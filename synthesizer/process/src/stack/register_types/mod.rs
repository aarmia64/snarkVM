// Copyright (C) 2019-2023 Aleo Systems Inc.
// This file is part of the snarkVM library.

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at:
// http://www.apache.org/licenses/LICENSE-2.0

// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

mod initialize;
mod matches;

use console::{
    network::prelude::*,
    program::{
        Access,
        EntryType,
        Identifier,
        LiteralType,
        PlaintextType,
        RecordType,
        Register,
        RegisterType,
        StructType,
        ValueType,
    },
};
use synthesizer_program::{
    CallOperator,
    Closure,
    Function,
    Instruction,
    InstructionTrait,
    Opcode,
    Operand,
    Program,
    StackMatches,
    StackProgram,
};

use indexmap::IndexMap;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct RegisterTypes<N: Network> {
    /// The mapping of all input registers to their defined types.
    inputs: IndexMap<u64, RegisterType<N>>,
    /// The mapping of all destination registers to their defined types.
    destinations: IndexMap<u64, RegisterType<N>>,
}

impl<N: Network> RegisterTypes<N> {
    /// Initializes a new instance of `RegisterTypes` for the given closure.
    /// Checks that the given closure is well-formed for the given stack.
    #[inline]
    pub fn from_closure(stack: &(impl StackMatches<N> + StackProgram<N>), closure: &Closure<N>) -> Result<Self> {
        Self::initialize_closure_types(stack, closure)
    }

    /// Initializes a new instance of `RegisterTypes` for the given function.
    /// Checks that the given function is well-formed for the given stack.
    #[inline]
    pub fn from_function(stack: &(impl StackMatches<N> + StackProgram<N>), function: &Function<N>) -> Result<Self> {
        Self::initialize_function_types(stack, function)
    }

    /// Returns `true` if the given register exists.
    pub fn contains(&self, register: &Register<N>) -> bool {
        // Retrieve the register locator.
        let locator = &register.locator();
        // The input and destination registers represent the full set of registers.
        // The output registers represent a subset of registers that are returned by the function.
        self.inputs.contains_key(locator) || self.destinations.contains_key(locator)
    }

    /// Returns `true` if the given register corresponds to an input register.
    pub fn is_input(&self, register: &Register<N>) -> bool {
        self.inputs.contains_key(&register.locator())
    }

    /// Returns the register type of the given operand.
    pub fn get_type_from_operand(
        &self,
        stack: &(impl StackMatches<N> + StackProgram<N>),
        operand: &Operand<N>,
    ) -> Result<RegisterType<N>> {
        Ok(match operand {
            Operand::Literal(literal) => RegisterType::Plaintext(PlaintextType::from(literal.to_type())),
            Operand::Register(register) => self.get_type(stack, register)?,
            Operand::ProgramID(_) => RegisterType::Plaintext(PlaintextType::Literal(LiteralType::Address)),
            Operand::Caller => RegisterType::Plaintext(PlaintextType::Literal(LiteralType::Address)),
            Operand::BlockHeight => bail!("'block.height' is not a valid operand in a non-finalize context."),
        })
    }

    /// Returns the register type of the given register.
    pub fn get_type(
        &self,
        stack: &(impl StackMatches<N> + StackProgram<N>),
        register: &Register<N>,
    ) -> Result<RegisterType<N>> {
        // Initialize a tracker for the register type.
        let register_type = if self.is_input(register) {
            // Retrieve the input value type as a register type.
            self.inputs.get(&register.locator()).ok_or_else(|| anyhow!("Register '{register}' does not exist"))?
        } else {
            // Retrieve the destination register type.
            self.destinations.get(&register.locator()).ok_or_else(|| anyhow!("Register '{register}' does not exist"))?
        };

        // Retrieve the path if the register is an access. Otherwise, return the register type.
        let mut path_iter = match &register {
            // If the register is a locator, then output the register type.
            Register::Locator(..) => return Ok(register_type.clone()),
            // If the register is an access, then traverse the path to output the register type.
            Register::Access(_, path) => {
                // Ensure the path is valid.
                ensure!(!path.is_empty(), "Register '{register}' references no accesses.");
                // Output the path.
                path
            }
        }
        .iter();

        // Because the register is an access, the accessed type must be a plaintext type.
        // We perform a single access, if the register type is a record.
        // This is done to minimize the number of `clone` operations.
        let mut plaintext_type = &match register_type {
            RegisterType::Plaintext(plaintext_type) => plaintext_type.clone(),
            RegisterType::Record(record_name) => {
                // Ensure the record type exists.
                ensure!(stack.program().contains_record(record_name), "Record '{record_name}' does not exist");
                // Retrieve the first access.
                // Note: this unwrap is safe since the path is checked to be non-empty above.
                let access = path_iter.next().unwrap();
                // Retrieve the member type from the record.
                if access == &Access::Member(Identifier::from_str("owner")?) {
                    // If the member is the owner, then output the address type.
                    PlaintextType::Literal(LiteralType::Address)
                } else {
                    // Retrieve the path name.
                    let path_name = match access {
                        Access::Member(path_name) => path_name,
                        Access::Index(_) => bail!("Attempted to index into a record"),
                    };
                    // Retrieve the entry type from the record.
                    match stack.program().get_record(record_name)?.entries().get(path_name) {
                        // Retrieve the plaintext type.
                        Some(entry_type) => entry_type.plaintext_type().clone(),
                        None => bail!("'{path_name}' does not exist in record '{record_name}'"),
                    }
                }
            }
            RegisterType::ExternalRecord(locator) => {
                // Ensure the external record type exists.
                ensure!(stack.contains_external_record(locator), "External record '{locator}' does not exist");
                // Retrieve the first access.
                // Note: this unwrap is safe since the path is checked to be non-empty above.
                let access = path_iter.next().unwrap();
                // Retrieve the member type from the external record.
                if access == &Access::Member(Identifier::from_str("owner")?) {
                    // If the member is the owner, then output the address type.
                    PlaintextType::Literal(LiteralType::Address)
                } else {
                    // Retrieve the path name.
                    let path_name = match access {
                        Access::Member(path_name) => path_name,
                        Access::Index(_) => bail!("Attempted to index into an external record"),
                    };
                    // Retrieve the entry type from the external record.
                    match stack.get_external_record(locator)?.entries().get(path_name) {
                        // Retrieve the plaintext type.
                        Some(entry_type) => entry_type.plaintext_type().clone(),
                        None => bail!("'{path_name}' does not exist in external record '{locator}'"),
                    }
                }
            }
        };

        // Traverse the path to find the register type.
        for access in path_iter {
            // Update the plaintext type at each step.
            match (plaintext_type, access) {
                // Ensure the plaintext type is not a literal, as the register references an access.
                (PlaintextType::Literal(..), _) => bail!("'{register}' references a literal."),
                // Traverse the path to output the register type.
                (PlaintextType::Struct(struct_name), Access::Member(identifier)) => {
                    // Retrieve the member type from the struct.
                    match stack.program().get_struct(struct_name)?.members().get(identifier) {
                        // Update the member type.
                        Some(member_type) => plaintext_type = member_type,
                        None => bail!("'{identifier}' does not exist in struct '{struct_name}'"),
                    }
                }
                // Traverse the path to output the register type.
                (PlaintextType::Array(array_type), Access::Index(index)) => match index < array_type.length() {
                    true => plaintext_type = array_type.next_element_type(),
                    false => bail!("'{index}' is out of bounds for '{register}'"),
                },
                (PlaintextType::Struct(..), Access::Index(..)) | (PlaintextType::Array(..), Access::Member(..)) => {
                    bail!("Invalid access `{access}`")
                }
            }
        }

        // Output the register type.
        Ok(RegisterType::Plaintext(plaintext_type.clone()))
    }
}
