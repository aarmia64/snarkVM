// Copyright (C) 2019-2021 Aleo Systems Inc.
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

use snarkvm_algorithms::{crh::sha256::sha256, SNARK, SRS};
use snarkvm_dpc::{DPCError, InnerCircuit, Network, OuterCircuit, ProgramExecutable, ProgramScheme};
use snarkvm_parameters::{
    testnet1::{InnerProvingKeyBytes, InnerVerifyingKeyBytes},
    traits::Parameter,
};
use snarkvm_utilities::{FromBytes, ToBytes};

use rand::thread_rng;
use std::path::PathBuf;

mod utils;
use utils::store;

pub fn setup<N: Network>() -> Result<(Vec<u8>, Vec<u8>), DPCError> {
    let rng = &mut thread_rng();

    let inner_proving_key: <N::InnerSNARK as SNARK>::ProvingKey =
        <N::InnerSNARK as SNARK>::ProvingKey::read_le(InnerProvingKeyBytes::load_bytes()?.as_slice())?;

    let inner_verifying_key: <N::InnerSNARK as SNARK>::VerifyingKey =
        <N::InnerSNARK as SNARK>::VerifyingKey::read_le(InnerVerifyingKeyBytes::load_bytes()?.as_slice())?;

    let inner_snark_proof = N::InnerSNARK::prove(&inner_proving_key, &InnerCircuit::<N>::blank(), rng)?;

    let noop_program = N::noop_program();
    let noop_executable = noop_program.to_executable(N::noop_circuit_id())?;

    let outer_snark_parameters = N::OuterSNARK::setup(
        &OuterCircuit::<N>::blank(
            inner_verifying_key,
            inner_snark_proof,
            noop_executable.execute(Default::default())?,
        ),
        &mut SRS::CircuitSpecific(rng),
    )?;

    let outer_snark_pk = outer_snark_parameters.0.to_bytes_le()?;
    let outer_snark_vk: <N::OuterSNARK as SNARK>::VerifyingKey = outer_snark_parameters.1.into();
    let outer_snark_vk = outer_snark_vk.to_bytes_le()?;

    println!("outer_snark_pk.params\n\tsize - {}", outer_snark_pk.len());
    println!("outer_snark_vk.params\n\tsize - {}", outer_snark_vk.len());
    Ok((outer_snark_pk, outer_snark_vk))
}

fn versioned_filename(checksum: &str) -> String {
    match checksum.get(0..7) {
        Some(sum) => format!("outer_snark_pk-{}.params", sum),
        _ => "outer_snark_pk.params".to_string(),
    }
}

pub fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        println!("Invalid number of arguments. Given: {} - Required: 1", args.len() - 1);
        return;
    }

    let (outer_snark_pk, outer_snark_vk) = match args[1].as_str() {
        "testnet1" => setup::<snarkvm_dpc::testnet1::Testnet1>().unwrap(),
        "testnet2" => setup::<snarkvm_dpc::testnet2::Testnet2>().unwrap(),
        _ => panic!("Invalid parameters"),
    };

    let outer_snark_pk_checksum = hex::encode(sha256(&outer_snark_pk));
    store(
        &PathBuf::from(&versioned_filename(&outer_snark_pk_checksum)),
        &PathBuf::from("outer_snark_pk.checksum"),
        &outer_snark_pk,
    )
    .unwrap();
    store(
        &PathBuf::from("outer_snark_vk.params"),
        &PathBuf::from("outer_snark_vk.checksum"),
        &outer_snark_vk,
    )
    .unwrap();
}
