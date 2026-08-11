//! Reverse queries for NodeGraph interface-input connections.

use std::collections::HashMap;

use anyhow::Result;

use crate::{sdf, usd};

use super::{Connectable, Input, Material, NodeGraph, Shader};

/// A NodeGraph's interface inputs and the inputs that consume their values.
///
/// Entries follow the container's composed input order. Every authored
/// interface input has an entry, including inputs with no consumers.
#[derive(Clone, Default)]
pub struct InterfaceInputConsumersMap {
    entries: Vec<InterfaceInputConsumers>,
}

impl InterfaceInputConsumersMap {
    /// The number of interface inputs represented by the map.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the container has no authored interface inputs.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The consumers for `interface_input`, or `None` when it is not an entry.
    pub fn consumers(&self, interface_input: &Input) -> Option<&[Input]> {
        self.entry(interface_input.path())
            .map(|entry| entry.consumers.as_slice())
    }

    /// Iterate over interface inputs and their consumers in composed order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&Input, &[Input])> {
        self.entries
            .iter()
            .map(|entry| (&entry.interface_input, entry.consumers.as_slice()))
    }

    fn entry(&self, path: &sdf::Path) -> Option<&InterfaceInputConsumers> {
        self.entries.iter().find(|entry| entry.interface_input.path() == path)
    }

    fn entry_mut(&mut self, path: &sdf::Path) -> Option<&mut InterfaceInputConsumers> {
        self.entries
            .iter_mut()
            .find(|entry| entry.interface_input.path() == path)
    }
}

#[derive(Clone)]
struct InterfaceInputConsumers {
    interface_input: Input,
    consumers: Vec<Input>,
}

/// The interface-input queries shared by [`NodeGraph`] and [`Material`].
///
/// A non-transitive map reports shader inputs and nested NodeGraph inputs that
/// connect directly to this container's interface. A transitive map follows
/// nested NodeGraph inputs to their leaf shader inputs. A nested interface
/// input with no consumers remains in the transitive result.
pub trait NodeGraphInterface: Connectable {
    /// Compute the reverse map from interface inputs to their consumers.
    ///
    /// This walks only the namespace subtree rooted at this container. It is
    /// the Rust counterpart of
    /// `UsdShadeNodeGraph::ComputeInterfaceInputConsumersMap`.
    fn compute_interface_input_consumers_map(&self, transitive: bool) -> Result<InterfaceInputConsumersMap> {
        let direct = compute_direct_map(self.prim())?;
        if !transitive {
            return Ok(direct);
        }

        let mut nested_maps = HashMap::new();
        collect_nested_maps(&direct, self.stage(), &mut nested_maps)?;
        if nested_maps.is_empty() {
            return Ok(direct);
        }

        let mut resolved = InterfaceInputConsumersMap::default();
        for (interface_input, consumers) in direct.iter() {
            let mut resolved_consumers = Vec::new();
            for consumer in consumers {
                resolve_consumer(consumer, &nested_maps, &mut resolved_consumers);
            }
            resolved.entries.push(InterfaceInputConsumers {
                interface_input: interface_input.clone(),
                consumers: resolved_consumers,
            });
        }
        Ok(resolved)
    }
}

impl NodeGraphInterface for NodeGraph {}
impl NodeGraphInterface for Material {}

fn compute_direct_map(root: &usd::Prim) -> Result<InterfaceInputConsumersMap> {
    let mut result = InterfaceInputConsumersMap {
        entries: connectable_inputs(root)?
            .unwrap_or_default()
            .into_iter()
            .map(|interface_input| InterfaceInputConsumers {
                interface_input,
                consumers: Vec::new(),
            })
            .collect(),
    };

    let mut stack = root.children()?;
    stack.reverse();
    while let Some(prim) = stack.pop() {
        let mut children = prim.children()?;
        children.reverse();
        stack.extend(children);

        let Some(inputs) = connectable_inputs(&prim)? else {
            continue;
        };
        for input in inputs {
            for source in input.connected_sources()?.sources() {
                if source.source_type() != super::AttributeType::Input || source.source_prim().path() != root.path() {
                    continue;
                }
                if let Some(entry) = result.entry_mut(source.source_path()) {
                    entry.consumers.push(input.clone());
                }
            }
        }
    }
    Ok(result)
}

fn connectable_inputs(prim: &usd::Prim) -> Result<Option<Vec<Input>>> {
    let stage = prim.stage();
    let path = prim.path();
    if Shader::get(stage, path.clone())?.is_none()
        && NodeGraph::get(stage, path.clone())?.is_none()
        && Material::get(stage, path.clone())?.is_none()
    {
        return Ok(None);
    }
    Ok(Some(
        prim.authored_attributes()?
            .into_iter()
            .filter_map(Input::from_attribute)
            .collect(),
    ))
}

fn collect_nested_maps(
    consumers: &InterfaceInputConsumersMap,
    stage: &usd::Stage,
    maps: &mut HashMap<sdf::Path, InterfaceInputConsumersMap>,
) -> Result<()> {
    let nested_paths: Vec<sdf::Path> = consumers
        .iter()
        .flat_map(|(_, consumers)| consumers)
        .map(|consumer| consumer.prim().path().clone())
        .collect();

    for path in nested_paths {
        if maps.contains_key(&path) || NodeGraph::get(stage, path.clone())?.is_none() {
            continue;
        }
        let nested = compute_direct_map(&stage.prim(path.clone()))?;
        maps.insert(path, nested.clone());
        collect_nested_maps(&nested, stage, maps)?;
    }
    Ok(())
}

fn resolve_consumer(
    consumer: &Input,
    nested_maps: &HashMap<sdf::Path, InterfaceInputConsumersMap>,
    resolved: &mut Vec<Input>,
) {
    let prim_path = consumer.prim().path().clone();
    let Some(map) = nested_maps.get(&prim_path) else {
        resolved.push(consumer.clone());
        return;
    };
    let Some(consumers) = map.consumers(consumer) else {
        return;
    };
    if consumers.is_empty() {
        resolved.push(consumer.clone());
        return;
    }
    for nested in consumers {
        resolve_consumer(nested, nested_maps, resolved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schemas::shade::Shader;

    fn paths(inputs: &[Input]) -> Vec<&str> {
        inputs.iter().map(|input| input.path().as_str()).collect()
    }

    #[test]
    fn direct_consumers() -> Result<()> {
        let stage = usd::Stage::builder().in_memory("anon.usda")?;
        let graph = NodeGraph::define(&stage, "/Graph")?;
        let gain = graph.create_input("gain", "float")?;
        let unused = graph.create_input("unused", "float")?;
        let shader = Shader::define(&stage, "/Graph/Shader")?;
        shader.create_input("gain", "float")?.connect_to_input(&gain)?;
        let nested = NodeGraph::define(&stage, "/Graph/Nested")?;
        nested.create_input("gain", "float")?.connect_to_input(&gain)?;

        let map = graph.compute_interface_input_consumers_map(false)?;
        assert_eq!(map.len(), 2);
        assert_eq!(
            paths(map.consumers(&gain).expect("gain entry")),
            ["/Graph/Shader.inputs:gain", "/Graph/Nested.inputs:gain"]
        );
        assert!(map.consumers(&unused).expect("unused entry").is_empty());
        Ok(())
    }

    #[test]
    fn transitive_consumers() -> Result<()> {
        let stage = usd::Stage::builder().in_memory("anon.usda")?;
        let graph = NodeGraph::define(&stage, "/Graph")?;
        let gain = graph.create_input("gain", "float")?;
        let spare = graph.create_input("spare", "float")?;
        let nested = NodeGraph::define(&stage, "/Graph/Nested")?;
        let nested_gain = nested.create_input("gain", "float")?.connect_to_input(&gain)?;
        let nested_spare = nested.create_input("spare", "float")?.connect_to_input(&spare)?;
        let shader = Shader::define(&stage, "/Graph/Nested/Shader")?;
        shader.create_input("gain", "float")?.connect_to_input(&nested_gain)?;

        let map = graph.compute_interface_input_consumers_map(true)?;
        assert_eq!(
            paths(map.consumers(&gain).expect("gain entry")),
            ["/Graph/Nested/Shader.inputs:gain"]
        );
        assert_eq!(
            paths(map.consumers(&spare).expect("spare entry")),
            [nested_spare.path().as_str()]
        );
        Ok(())
    }

    #[test]
    fn material_consumers() -> Result<()> {
        let stage = usd::Stage::builder().in_memory("anon.usda")?;
        let material = Material::define(&stage, "/Mat")?;
        let roughness = material.create_input("roughness", "float")?;
        let shader = Shader::define(&stage, "/Mat/Surface")?;
        shader
            .create_input("roughness", "float")?
            .connect_to_input(&roughness)?;

        let map = material.compute_interface_input_consumers_map(false)?;
        assert_eq!(
            paths(map.consumers(&roughness).expect("roughness entry")),
            ["/Mat/Surface.inputs:roughness"]
        );
        Ok(())
    }
}
