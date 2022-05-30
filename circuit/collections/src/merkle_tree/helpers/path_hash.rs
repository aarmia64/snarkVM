// Copyright (C) 2019-2022 Aleo Systems Inc.
// This file is part of the snarkVM library.

// The snarkVM library is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// The snarkVM library is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with the snarkVM library. If not, see <https://www.gnu.org/licenses/>.

use super::*;
use snarkvm_circuit_algorithms::{Hash, Poseidon, BHP};

/// A trait for a Merkle path hash function.
pub trait PathHash<E: Environment> {
    /// Returns the hash of the given child nodes.
    fn hash_children(&self, left: &Field<E>, right: &Field<E>) -> Field<E>;

    /// Returns the empty hash.
    fn hash_empty(&self) -> Field<E> {
        self.hash_children(&Field::zero(), &Field::zero())
    }
}

impl<E: Environment, const NUM_WINDOWS: u8, const WINDOW_SIZE: u8> PathHash<E> for BHP<E, NUM_WINDOWS, WINDOW_SIZE> {
    /// Returns the hash of the given child nodes.
    fn hash_children(&self, left: &Field<E>, right: &Field<E>) -> Field<E> {
        // Prepend the nodes with a `true` bit.
        let mut input = vec![Boolean::constant(true)];
        input.extend(left.to_bits_le());
        input.extend(right.to_bits_le());
        // Hash the input.
        Hash::hash(self, &input)
    }
}

impl<E: Environment, const RATE: usize> PathHash<E> for Poseidon<E, RATE> {
    /// Returns the hash of the given child nodes.
    fn hash_children(&self, left: &Field<E>, right: &Field<E>) -> Field<E> {
        // Prepend the nodes with a `1field` byte.
        let mut input = vec![Field::one()];
        input.push(left.clone());
        input.push(right.clone());
        // Hash the input.
        Hash::hash(self, &input)
    }
}

#[cfg(all(test, console))]
mod tests {
    use super::*;
    use snarkvm_circuit_algorithms::{Poseidon2, BHP512};
    use snarkvm_circuit_types::environment::Circuit;
    use snarkvm_utilities::{test_rng, UniformRand};

    use anyhow::Result;

    const ITERATIONS: u64 = 10;
    const DOMAIN: &str = "MerkleTreeCircuit0";

    macro_rules! check_hash_children {
        ($hash:ident, $form:ident, $mode:ident, ($num_constants:expr, $num_public:expr, $num_private:expr, $num_constraints:expr)) => {{
            // Initialize the hash.
            let native = snarkvm_console_algorithms::$hash::<<Circuit as Environment>::$form>::setup(DOMAIN)?;
            let circuit = $hash::<Circuit>::constant(native.clone());

            for i in 0..ITERATIONS {
                // Sample a random input.
                let left = <Circuit as Environment>::BaseField::rand(&mut test_rng());
                let right = <Circuit as Environment>::BaseField::rand(&mut test_rng());

                // Compute the expected hash.
                let expected: <Circuit as Environment>::BaseField = console::merkle_tree::PathHash::<
                    <Circuit as Environment>::BaseField,
                >::hash_children(&native, &left, &right)
                .expect("Failed to hash native input");

                // Prepare the circuit input.
                let left = Field::new(Mode::$mode, left);
                let right = Field::new(Mode::$mode, right);

                Circuit::scope(format!("PathHash {i}"), || {
                    // Perform the hash operation.
                    let candidate = circuit.hash_children(&left, &right);
                    assert_scope!($num_constants, $num_public, $num_private, $num_constraints);
                    assert_eq!(expected, candidate.eject_value());
                });
                Circuit::reset();
            }
            Ok::<_, anyhow::Error>(())
        }};
    }

    #[test]
    fn test_hash_children_bhp512_constant() -> Result<()> {
        check_hash_children!(BHP512, Affine, Constant, (1611, 0, 0, 0))
    }

    #[test]
    fn test_hash_children_bhp512_public() -> Result<()> {
        check_hash_children!(BHP512, Affine, Public, (421, 0, 1385, 1387))
    }

    #[test]
    fn test_hash_children_bhp512_private() -> Result<()> {
        check_hash_children!(BHP512, Affine, Private, (421, 0, 1385, 1387))
    }

    #[test]
    fn test_hash_children_poseidon2_constant() -> Result<()> {
        check_hash_children!(Poseidon2, BaseField, Constant, (1, 0, 0, 0))
    }

    #[test]
    fn test_hash_children_poseidon2_public() -> Result<()> {
        check_hash_children!(Poseidon2, BaseField, Public, (1, 0, 540, 540))
    }

    #[test]
    fn test_hash_children_poseidon2_private() -> Result<()> {
        check_hash_children!(Poseidon2, BaseField, Private, (1, 0, 540, 540))
    }
}
