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

#![allow(non_snake_case)]

use crate::{
    fft::{EvaluationDomain, Evaluations as EvaluationsOnDomain},
    polycommit::sonic_pc::LabeledPolynomial,
    r1cs::{ConstraintSystem, Index as VarIndex},
    snark::varuna::{
        ahp::{indexer::Matrix, AHPForR1CS, CircuitId},
        VarunaHidingMode,
    },
};
use snarkvm_fields::{Field, PrimeField};
use snarkvm_utilities::{cfg_iter, cfg_iter_mut, serialize::*};

use anyhow::{ensure, Result};
use itertools::Itertools;
use std::collections::BTreeMap;

#[cfg(not(feature = "serial"))]
use rayon::prelude::*;

// This function converts a matrix output by Zexe's constraint infrastructure
// to the one used in this crate.
pub(crate) fn to_matrix_helper<F: Field>(matrix: &[Vec<(F, VarIndex)>], num_input_variables: usize) -> Matrix<F> {
    cfg_iter!(matrix)
        .map(|row| {
            let mut row_map = BTreeMap::new();
            row.iter().for_each(|(val, column)| {
                if !val.is_zero() {
                    let column = match column {
                        VarIndex::Public(i) => *i,
                        VarIndex::Private(i) => num_input_variables + i,
                    };
                    *row_map.entry(column).or_insert_with(F::zero) += *val;
                }
            });
            row_map.into_iter().map(|(column, val)| (val, column)).collect()
        })
        .collect()
}

/// Adds variables to randomize each z_M and preserve zero-knowledge
/// When no random assignments are passed, we use F::one()
pub(crate) fn add_randomizing_variables<F: PrimeField, CS: ConstraintSystem<F>>(
    cs: &mut CS,
    rand_assignments: Option<[F; 3]>,
) {
    let mut assignments = [F::one(); 3];
    if let Some(r) = rand_assignments {
        assignments = r;
    }

    let zk_vars = assignments
        .into_iter()
        .enumerate()
        .map(|(i, assignment)| cs.alloc(|| format!("random_{i}"), || Ok(assignment)).unwrap())
        .collect_vec();
    cs.enforce(|| "constraint zk", |lc| lc + zk_vars[0], |lc| lc + zk_vars[1], |lc| lc + zk_vars[2]);
}

/// Pads the public variables up to the closest power of two.
pub(crate) fn pad_input_for_indexer_and_prover<F: PrimeField, CS: ConstraintSystem<F>>(cs: &mut CS) {
    let num_public_variables = cs.num_public_variables();

    let power_of_two = EvaluationDomain::<F>::new(num_public_variables);
    assert!(power_of_two.is_some());

    // Allocated `zero` variables to pad the public input up to the next power of two.
    let padded_size = power_of_two.unwrap().size();
    if padded_size > num_public_variables {
        for i in 0..(padded_size - num_public_variables) {
            cs.alloc_input(|| format!("pad_input_{i}"), || Ok(F::zero())).unwrap();
        }
    }
}

#[derive(Clone, Debug, CanonicalSerialize, CanonicalDeserialize, PartialEq, Eq)]
pub struct MatrixEvals<F: PrimeField> {
    /// Evaluations of the `row` polynomial.
    pub row: EvaluationsOnDomain<F>,
    /// Evaluations of the `col` polynomial.
    pub col: EvaluationsOnDomain<F>,
    /// Evaluations of the `row_col` polynomial.
    /// After indexing, we drop these evaluations to save space in the ProvingKey
    pub row_col: Option<EvaluationsOnDomain<F>>,
    /// Evaluations of the `row_col_val` polynomial.
    pub row_col_val: EvaluationsOnDomain<F>,
}

impl<F: PrimeField> MatrixEvals<F> {
    pub(crate) fn evaluate(&self, lagrange_coefficients_at_point: &[F]) -> Result<[F; 4]> {
        ensure!(self.row_col.is_some(), "row_col evaluations are not available");
        Ok([
            self.row.evaluate_with_coeffs(lagrange_coefficients_at_point),
            self.col.evaluate_with_coeffs(lagrange_coefficients_at_point),
            self.row_col.as_ref().unwrap().evaluate_with_coeffs(lagrange_coefficients_at_point),
            self.row_col_val.evaluate_with_coeffs(lagrange_coefficients_at_point),
        ])
    }
}

/// Contains information about the arithmetization of the matrices, as per the Aleo protocol docs
#[derive(Clone, Debug, CanonicalSerialize, CanonicalDeserialize, PartialEq, Eq)]
pub struct MatrixArithmetization<F: PrimeField> {
    /// LDE of the row indices of M^*.
    pub row: LabeledPolynomial<F>,
    /// LDE of the column indices of M^*.
    pub col: LabeledPolynomial<F>,
    /// LDE of the vector containing entry-wise products of `row` and `col`.
    pub row_col: LabeledPolynomial<F>,
    /// LDE of the vector containing entry-wise products of `row`, `col` and the non-zero entries of M.
    pub row_col_val: LabeledPolynomial<F>,

    /// Evaluation of the above polynomials on the domain `K`.
    pub evals_on_K: MatrixEvals<F>,
}

pub(crate) fn matrix_evals<F: PrimeField>(
    matrix: &Matrix<F>,
    non_zero_domain: &EvaluationDomain<F>,
    variable_domain: &EvaluationDomain<F>,
    input_domain: &EvaluationDomain<F>,
    constraint_domain_elems: &[F],
    variable_domain_elems: &[F],
) -> Result<MatrixEvals<F>> {
    let lde_evals_time = start_timer!(|| "Computing row, col and val evals");

    // We are computing the arithmetization of M,
    // where `M(α,β) = \sum_{κ∈K} val(κ)·L^R_row(κ)(α)·L^C_col(κ)(β)`

    let mut row_indices = Vec::with_capacity(non_zero_domain.size());
    let mut col_indices = Vec::with_capacity(non_zero_domain.size());
    let mut row_col_indices = Vec::with_capacity(non_zero_domain.size());
    let mut row_col_vals = Vec::with_capacity(non_zero_domain.size());

    for (row_index, row) in matrix.iter().enumerate() {
        for (val, input_var_index) in row {
            let row_i = constraint_domain_elems[row_index];
            let col_i = variable_domain_elems[variable_domain.reindex_by_subdomain(input_domain, *input_var_index)?];

            row_indices.push(row_i);
            row_col_indices.push(row_i);
            col_indices.push(col_i);
            row_col_vals.push(*val);
        }
    }

    let non_zero_entries = row_indices.len();

    // Zip safety: we intentionally only multiply the first non_zero_entries
    cfg_iter_mut!(row_col_indices).zip(&col_indices).for_each(|(rc, &col)| *rc *= col);
    cfg_iter_mut!(row_col_vals).zip(&row_col_indices).for_each(|(v, rc)| *v *= rc);

    // Fill up the evaluations to the next power of two
    let padding = non_zero_domain.size() - non_zero_entries;
    for _ in 0..padding {
        row_indices.push(F::one());
        col_indices.push(F::one());
        row_col_indices.push(F::one());
        row_col_vals.push(F::zero());
    }

    end_timer!(lde_evals_time);

    let row_evals_on_K = EvaluationsOnDomain::from_vec_and_domain(row_indices, *non_zero_domain);
    let col_evals_on_K = EvaluationsOnDomain::from_vec_and_domain(col_indices, *non_zero_domain);
    let row_col_evals_on_K = EvaluationsOnDomain::from_vec_and_domain(row_col_indices, *non_zero_domain);
    let row_col_val_evals_on_K = EvaluationsOnDomain::from_vec_and_domain(row_col_vals, *non_zero_domain);
    Ok(MatrixEvals {
        row: row_evals_on_K,
        col: col_evals_on_K,
        row_col: Some(row_col_evals_on_K),
        row_col_val: row_col_val_evals_on_K,
    })
}

// TODO for debugging: add test that checks result of arithmetize_matrix(M).
pub(crate) fn arithmetize_matrix<F: PrimeField>(
    id: &CircuitId,
    label: &str,
    matrix_evals: MatrixEvals<F>,
) -> Result<MatrixArithmetization<F>> {
    ensure!(matrix_evals.row_col.is_some(), "row_col evaluations are not available");

    let interpolate_time = start_timer!(|| "Interpolating on K");
    let row = matrix_evals.row.clone().interpolate();
    let col = matrix_evals.col.clone().interpolate();
    let row_col = matrix_evals.row_col.as_ref().unwrap().clone().interpolate();
    let row_col_val = matrix_evals.row_col_val.clone().interpolate();
    end_timer!(interpolate_time);

    let label = &[label];
    let mut labels = AHPForR1CS::<F, VarunaHidingMode>::index_polynomial_labels(label, std::iter::once(id));

    Ok(MatrixArithmetization {
        row: LabeledPolynomial::new(labels.next().unwrap(), row, None, None),
        col: LabeledPolynomial::new(labels.next().unwrap(), col, None, None),
        row_col: LabeledPolynomial::new(labels.next().unwrap(), row_col, None, None),
        row_col_val: LabeledPolynomial::new(labels.next().unwrap(), row_col_val, None, None),
        evals_on_K: matrix_evals,
    })
}

/// Compute the transpose of a sparse matrix
pub(crate) fn transpose<F: PrimeField>(
    matrix: &Matrix<F>,
    variable_domain: &EvaluationDomain<F>,
    input_domain: &EvaluationDomain<F>,
) -> Result<Matrix<F>> {
    // NOTE: we cannot preallocate the inner Vec because we don't know ahead of time how many rows are used by which variables
    let mut transpose = vec![vec![]; variable_domain.size()];
    for (row_index, row) in matrix.iter().enumerate() {
        for (val, input_var_index) in row {
            let c_i = variable_domain.reindex_by_subdomain(input_domain, *input_var_index)?;
            transpose[c_i].push((*val, row_index));
        }
    }
    Ok(transpose)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snark::varuna::num_non_zero;
    use snarkvm_curves::bls12_377::Fr as F;
    use snarkvm_fields::{One, Zero};
    use std::{borrow::Cow, collections::HashMap};

    fn entry(matrix: &Matrix<F>, row: usize, col: usize) -> F {
        matrix[row].iter().find_map(|(f, i)| (i == &col).then_some(*f)).unwrap_or_else(F::zero)
    }

    #[test]
    fn check_arithmetization() {
        let a = vec![
            vec![(F::one(), 1), (F::one(), 2)],
            vec![(F::one(), 3)],
            vec![(F::one(), 3)],
            vec![(F::one(), 0), (F::one(), 1), (F::one(), 5)],
            vec![(F::one(), 1), (F::one(), 2), (F::one(), 6)],
            vec![(F::one(), 2), (F::one(), 5), (F::one(), 7)],
            vec![(F::one(), 3), (F::one(), 4), (F::one(), 6)],
            vec![(F::one(), 0), (F::one(), 6), (F::one(), 7)],
        ];

        let b = vec![
            vec![],
            vec![(F::one(), 1)],
            vec![(F::one(), 0)],
            vec![(F::one(), 2)],
            vec![(F::one(), 3)],
            vec![(F::one(), 4)],
            vec![(F::one(), 5)],
            vec![(F::one(), 6)],
        ];

        let c = vec![vec![], vec![(F::one(), 7)], vec![], vec![], vec![], vec![(F::one(), 3)], vec![], vec![]];

        let constraint_domain = EvaluationDomain::new(2 + 6).unwrap();
        let variable_domain = EvaluationDomain::new(2 + 6).unwrap();
        let input_domain = EvaluationDomain::new(2).unwrap();
        let inverse_map = constraint_domain.elements().enumerate().map(|(i, e)| (e, i)).collect::<HashMap<_, _>>();
        let elements = constraint_domain.elements().collect::<Vec<_>>();
        let reindexed_inverse_map = (0..constraint_domain.size())
            .map(|i| {
                let reindexed_i = constraint_domain.reindex_by_subdomain(&input_domain, i).unwrap();
                (elements[reindexed_i], i)
            })
            .collect::<HashMap<_, _>>();
        let constraint_domain_elements = constraint_domain.elements().collect::<Vec<_>>();
        let variable_domain_elements = if variable_domain == constraint_domain {
            Cow::from(&constraint_domain_elements)
        } else {
            Cow::from(variable_domain.elements().collect::<Vec<_>>())
        };
        for (matrix, label) in [(a, "a"), (b, "b"), (c, "c")] {
            let num_non_zero = num_non_zero(&matrix);
            let interpolation_domain = EvaluationDomain::new(num_non_zero).unwrap();

            let evals = matrix_evals(
                &matrix,
                &interpolation_domain,
                &variable_domain,
                &input_domain,
                &constraint_domain_elements,
                &variable_domain_elements,
            )
            .unwrap();
            let dummy_id = CircuitId([0; 32]);
            let arith = arithmetize_matrix(&dummy_id, label, evals).unwrap();

            for (k_index, k) in interpolation_domain.elements().enumerate() {
                let row_val = arith.row.evaluate(k);
                let col_val = arith.col.evaluate(k);
                let row_col = arith.row_col.evaluate(k);

                let row_col_val = arith.row_col_val.evaluate(k);
                assert_eq!(arith.evals_on_K.row[k_index], row_val);
                assert_eq!(arith.evals_on_K.col[k_index], col_val);
                assert_eq!(arith.evals_on_K.row_col_val[k_index], row_col_val);
                if k_index < num_non_zero {
                    let col = *dbg!(reindexed_inverse_map.get(&col_val).unwrap());
                    let row = *dbg!(inverse_map.get(&row_val).unwrap());
                    assert!(matrix[row].iter().any(|(_, c)| *c == col));
                    assert_eq!(row_col_val, entry(&matrix, row, col) * row_col);
                }
            }
        }
    }
}
