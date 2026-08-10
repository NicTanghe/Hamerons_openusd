//! The UsdShade connectable interface shared across shading prims.

use anyhow::Result;

use crate::usd::SchemaBase;

use super::connectable::{input_name, output_name};
use super::tokens::{NS_INPUTS, NS_OUTPUTS};
use super::{Input, Output};

/// The `inputs:` and `outputs:` surface shared by Shader, NodeGraph, and
/// Material prims (C++ `UsdShadeConnectableAPI`).
pub trait Connectable: SchemaBase {
    /// The `inputs:<base>` attribute view (C++ `GetInput`).
    fn input(&self, base: &str) -> Input {
        Input::new(self.prim().attribute(input_name(base)))
    }

    /// Author `inputs:<base>` with `type_name` and `custom = false`.
    fn create_input(&self, base: &str, type_name: &str) -> Result<Input> {
        Ok(Input::new(
            self.prim()
                .create_attribute(input_name(base), type_name)?
                .set_custom(false)?,
        ))
    }

    /// The authored inputs in composed property order (C++ `GetInputs`).
    fn inputs(&self) -> Result<Vec<Input>> {
        Ok(self
            .prim()
            .authored_property_names()?
            .into_iter()
            .filter(|name| name.starts_with(NS_INPUTS))
            .map(|name| Input::new(self.prim().attribute(name)))
            .collect())
    }

    /// The `outputs:<base>` attribute view (C++ `GetOutput`).
    fn output(&self, base: &str) -> Output {
        Output::new(self.prim().attribute(output_name(base)))
    }

    /// Author `outputs:<base>` with `type_name` and `custom = false`.
    fn create_output(&self, base: &str, type_name: &str) -> Result<Output> {
        Ok(Output::new(
            self.prim()
                .create_attribute(output_name(base), type_name)?
                .set_custom(false)?,
        ))
    }

    /// The authored outputs in composed property order (C++ `GetOutputs`).
    fn outputs(&self) -> Result<Vec<Output>> {
        Ok(self
            .prim()
            .authored_property_names()?
            .into_iter()
            .filter(|name| name.starts_with(NS_OUTPUTS))
            .map(|name| Output::new(self.prim().attribute(name)))
            .collect())
    }
}
