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

use super::*;

impl<N: Network> RegisterTypes<N> {
    /// Checks that the given operands matches the layout of the struct. The ordering of the operands matters.
    pub fn matches_struct(
        &self,
        stack: &(impl StackMatches<N> + StackProgram<N>),
        operands: &[Operand<N>],
        struct_: &StructType<N>,
    ) -> Result<()> {
        // Retrieve the struct name.
        let struct_name = struct_.name();
        // Ensure the struct name is valid.
        ensure!(!Program::is_reserved_keyword(struct_name), "Struct name '{struct_name}' is reserved");

        // Ensure the operands length is at least the minimum required.
        if operands.len() < N::MIN_STRUCT_ENTRIES {
            bail!("'{struct_name}' must have at least {} operand(s)", N::MIN_STRUCT_ENTRIES)
        }
        // Ensure the number of struct members does not exceed the maximum.
        if operands.len() > N::MAX_STRUCT_ENTRIES {
            bail!("'{struct_name}' cannot exceed {} entries", N::MAX_STRUCT_ENTRIES)
        }

        // Ensure the number of struct members match.
        let num_members = operands.len();
        let expected_num_members = struct_.members().len();
        if expected_num_members != num_members {
            bail!("'{struct_name}' expected {expected_num_members} members, found {num_members} members")
        }

        // Ensure the operand types match the struct.
        for (operand, (member_name, member_type)) in operands.iter().zip_eq(struct_.members()) {
            match operand {
                // Ensure the literal type matches the member type.
                Operand::Literal(literal) => {
                    ensure!(
                        &PlaintextType::Literal(literal.to_type()) == member_type,
                        "Struct member '{struct_name}.{member_name}' expects a {member_type}, but found '{operand}' in the operand.",
                    )
                }
                // Ensure the register type matches the member type.
                Operand::Register(register) => {
                    // Retrieve the register type.
                    match self.get_type(stack, register)? {
                        // Ensure the register type is not a record.
                        RegisterType::ExternalRecord(..) | RegisterType::Record(..) => {
                            bail!("Casting a record into a record entry is illegal")
                        }
                        // Ensure the register type matches the member type.
                        RegisterType::Plaintext(type_) => {
                            ensure!(
                                &type_ == member_type,
                                "Record entry '{struct_name}.{member_name}' expects a '{member_type}', but found '{type_}' in the operand '{operand}'.",
                            )
                        }
                    }
                }
                // Ensure the program ID type (address) matches the member type.
                Operand::ProgramID(..) => {
                    // Retrieve the program ID type.
                    let program_ref_type = PlaintextType::Literal(LiteralType::Address);
                    // Ensure the program ID type matches the member type.
                    ensure!(
                        &program_ref_type == member_type,
                        "Struct member '{struct_name}.{member_name}' expects {member_type}, but found '{program_ref_type}' in the operand '{operand}'.",
                    )
                }
                // Ensure the caller type (address) matches the member type.
                Operand::Caller => {
                    // Retrieve the caller type.
                    let caller_type = PlaintextType::Literal(LiteralType::Address);
                    // Ensure the caller type matches the member type.
                    ensure!(
                        &caller_type == member_type,
                        "Struct member '{struct_name}.{member_name}' expects {member_type}, but found '{caller_type}' in the operand '{operand}'.",
                    )
                }
                // If the operand is a block height type, throw an error.
                Operand::BlockHeight => bail!(
                    "Struct member '{struct_name}.{member_name}' cannot be from a block height in a non-finalize scope"
                ),
            }
        }
        Ok(())
    }

    /// Checks that the given record matches the layout of the record type.
    /// Note: Ordering for `owner` **does** matter, however ordering
    /// for record data does **not** matter, as long as all defined members are present.
    pub fn matches_record(
        &self,
        stack: &(impl StackMatches<N> + StackProgram<N>),
        operands: &[Operand<N>],
        record_type: &RecordType<N>,
    ) -> Result<()> {
        // Retrieve the record name.
        let record_name = record_type.name();
        // Ensure the record name is valid.
        ensure!(!Program::is_reserved_keyword(record_name), "Record name '{record_name}' is reserved");

        // Ensure the operands length is at least the minimum required.
        if operands.len() < N::MIN_RECORD_ENTRIES {
            bail!("'{record_name}' must have at least {} operand(s)", N::MIN_RECORD_ENTRIES)
        }
        // Ensure the operands length is within the maximum limit.
        if operands.len() > N::MAX_RECORD_ENTRIES {
            bail!("'{record_name}' cannot exceed {} entries", N::MAX_RECORD_ENTRIES)
        }

        // Ensure the number of record entries match.
        let num_entries = operands.len().saturating_sub(N::MIN_RECORD_ENTRIES);
        let expected_num_entries = record_type.entries().len();
        if expected_num_entries != num_entries {
            bail!("'{record_name}' expected {expected_num_entries} entries, found {num_entries} entries")
        }

        // Ensure the first input type is an address.
        match &operands[0] {
            Operand::Literal(literal) => {
                ensure!(
                    literal.to_type() == LiteralType::Address,
                    "Casting to a record requires the first operand to be an address"
                )
            }
            Operand::Register(register) => {
                // Retrieve the register type.
                let register_type = self.get_type(stack, register)?;
                // Ensure the register type is an address.
                ensure!(
                    register_type == RegisterType::Plaintext(PlaintextType::Literal(LiteralType::Address)),
                    "Casting to a record requires the first operand to be an address"
                );
            }
            Operand::ProgramID(program_id) => {
                // Note: While the ProgramID is rendered as an address, this address is not recoverable
                // from a private key. Furthermore, programs are not allowed to own any records.
                // They must hold all necessary state in storage instead.
                bail!("Forbidden operation: Cannot cast a program ID ('{program_id}') as a record owner")
            }
            Operand::Caller => {}
            Operand::BlockHeight => {
                bail!("Forbidden operation: Cannot cast a block height as a record owner")
            }
        }

        // Ensure the operand types match the record entry types.
        for (operand, (entry_name, entry_type)) in
            operands.iter().skip(N::MIN_RECORD_ENTRIES).zip_eq(record_type.entries())
        {
            match entry_type {
                EntryType::Constant(plaintext_type)
                | EntryType::Public(plaintext_type)
                | EntryType::Private(plaintext_type) => {
                    match operand {
                        // Ensure the literal type matches the entry type.
                        Operand::Literal(literal) => {
                            ensure!(
                                &PlaintextType::Literal(literal.to_type()) == plaintext_type,
                                "Record entry '{record_name}.{entry_name}' expects a '{plaintext_type}', but found '{literal}' in the operand.",
                            )
                        }
                        // Ensure the register type matches the entry type.
                        Operand::Register(register) => {
                            // Retrieve the register type.
                            match self.get_type(stack, register)? {
                                // Ensure the register type is not a record.
                                RegisterType::ExternalRecord(..) | RegisterType::Record(..) => {
                                    bail!("Casting a record into a record entry is illegal")
                                }
                                // Ensure the register type matches the entry type.
                                RegisterType::Plaintext(type_) => {
                                    ensure!(
                                        &type_ == plaintext_type,
                                        "Record entry '{record_name}.{entry_name}' expects a '{plaintext_type}', but found '{type_}' in the operand '{operand}'.",
                                    )
                                }
                            }
                        }
                        // Ensure the program ID type (address) matches the entry type.
                        Operand::ProgramID(..) => {
                            // Retrieve the program ID type.
                            let program_ref_type = &PlaintextType::Literal(LiteralType::Address);
                            // Ensure the program ID type matches the entry type.
                            ensure!(
                                program_ref_type == plaintext_type,
                                "Record entry '{record_name}.{entry_name}' expects a '{plaintext_type}', but found '{program_ref_type}' in the operand '{operand}'.",
                            )
                        }
                        // Ensure the caller type (address) matches the entry type.
                        Operand::Caller => {
                            // Retrieve the caller type.
                            let caller_type = &PlaintextType::Literal(LiteralType::Address);
                            // Ensure the caller type matches the entry type.
                            ensure!(
                                caller_type == plaintext_type,
                                "Record entry '{record_name}.{entry_name}' expects a '{plaintext_type}', but found '{caller_type}' in the operand '{operand}'.",
                            )
                        }
                        // Fail if the operand is a block height.
                        Operand::BlockHeight => {
                            bail!(
                                "Record entry '{record_name}.{entry_name}' expects a '{plaintext_type}', but found a block height in the operand '{operand}'."
                            )
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
