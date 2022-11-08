// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::nearly_zero::NearlyZero;
use num_bigint::BigUint;
use num_complex::Complex64;
use num_traits::{One, Zero};
use rand::Rng;
use rustc_hash::FxHashMap;
use std::f64::consts::FRAC_1_SQRT_2;
use std::ops::ControlFlow;

pub type SparseState = FxHashMap<BigUint, Complex64>;

/// The `QuantumSim` struct contains the necessary state for tracking the simulation. Each instance of a
/// `QuantumSim` represents an independant simulation.
pub(crate) struct QuantumSim {
    /// The structure that describes the current quantum state.
    pub(crate) state: FxHashMap<BigUint, Complex64>,

    /// The mapping from qubit identifiers to internal state locations.
    pub(crate) id_map: FxHashMap<usize, usize>,
}

impl Default for QuantumSim {
    fn default() -> Self {
        Self::new()
    }
}

/// Provides the common set of functionality across all quantum simulation types.
impl QuantumSim {
    /// Creates a new sparse state quantum simulator object with empty initial state (no qubits allocated, no operations buffered).
    #[must_use]
    fn new() -> Self {
        QuantumSim {
            state: FxHashMap::default(),

            id_map: FxHashMap::default(),
        }
    }

    /// Allocates a fresh qubit, returning its identifier. Note that this will use the lowest available
    /// identifier, and may result in qubits being allocated "in the middle" of an existing register
    /// if those identifiers are available.
    #[must_use]
    pub(crate) fn allocate(&mut self) -> usize {
        if self.id_map.is_empty() {
            // Add the intial value for the zero state.
            self.state.insert(BigUint::zero(), Complex64::one());
        }

        // Add the new entry into the FxHashMap at the first available sequential ID.
        let mut sorted_keys: Vec<&usize> = self.id_map.keys().collect();
        sorted_keys.sort();
        let n_qubits = sorted_keys.len();
        let new_key = sorted_keys
            .iter()
            .enumerate()
            .take_while(|(index, key)| index == **key)
            .last()
            .map_or(0_usize, |(_, &&key)| key + 1);
        self.id_map.insert(new_key, n_qubits);

        // Return the new ID that was used.
        new_key
    }

    /// Releases the given qubit, collapsing its state in the process. After release that identifier is
    /// no longer valid for use in other functions and will cause an error if used.
    /// # Panics
    ///
    /// The function will panic if the given id does not correpsond to an allocated qubit.
    pub(crate) fn release(&mut self, id: usize) {
        // Since it is easier to release a contiguous half of the state, find the qubit
        // with the last location and swap that with the qubit to be released.
        let n_qubits = self.id_map.len();
        let loc = *self
            .id_map
            .get(&id)
            .unwrap_or_else(|| panic!("Unable to find qubit with id {}.", id));
        let last_loc = n_qubits - 1;
        if last_loc != loc {
            let last_id = *self
                .id_map
                .iter()
                .find(|(_, &value)| value == last_loc)
                .unwrap()
                .0;
            self.swap_qubit_state(loc, last_loc);
            *(self.id_map.get_mut(&last_id).unwrap()) = loc;
            *(self.id_map.get_mut(&id).unwrap()) = last_loc;
        };

        // Measure and collapse the state for this qubit.
        let res = self.measure_impl(last_loc);

        // Remove the released ID from the mapping and cleanup the unused part of the state.
        self.id_map.remove(&id);
        if res {
            let qubit = self.id_map.len() as u64;
            self.state = self
                .state
                .drain()
                .fold(FxHashMap::default(), |mut accum, (k, v)| {
                    let mut new_k = k.clone();
                    new_k.set_bit(qubit, !k.bit(qubit));
                    accum.insert(new_k, v);
                    accum
                });
        }
    }

    /// Prints the current state vector to standard output with integer labels for the states, skipping any
    /// states with zero amplitude.
    /// # Panics
    ///
    /// This function panics if it is unable sort the state into qubit id order.
    pub(crate) fn dump(&mut self) {
        // Swap all the entries in the state to be ordered by qubit identifier. This makes
        // interpreting the state easier for external consumers that don't have access to the id map.
        let mut sorted_keys: Vec<usize> = self.id_map.keys().copied().collect();
        sorted_keys.sort_unstable();
        sorted_keys.iter().enumerate().for_each(|(index, &key)| {
            if index != self.id_map[&key] {
                self.swap_qubit_state(self.id_map[&key], index);
                let swapped_key = *self
                    .id_map
                    .iter()
                    .find(|(_, &value)| value == index)
                    .unwrap()
                    .0;
                *(self.id_map.get_mut(&swapped_key).unwrap()) = self.id_map[&key];
                *(self.id_map.get_mut(&key).unwrap()) = index;
            }
        });

        self.dump_impl(false);
    }

    /// Utility function that performs the actual output of state (and optionally map) to screen. Can
    /// be called internally from other functions to aid in debugging and does not perform any modification
    /// of the internal structures.
    fn dump_impl(&self, print_id_map: bool) {
        if print_id_map {
            println!("MAP: {:?}", self.id_map);
        };
        print!("STATE: [ ");
        let mut sorted_keys = self.state.keys().collect::<Vec<_>>();
        sorted_keys.sort_unstable();
        for key in sorted_keys {
            print!(
                "|{}\u{27e9}: {}, ",
                key,
                self.state.get(key).map_or_else(Complex64::zero, |v| *v)
            );
        }
        println!("]");
    }

    /// Checks the probability of parity measurement in the computational basis for the given set of
    /// qubits.
    /// # Panics
    ///
    /// This function will panic if the given ids do not all correspond to allocated qubits.
    /// This function will panic if there are duplicate ids in the given list.
    #[must_use]
    pub(crate) fn joint_probability(&mut self, ids: &[usize]) -> f64 {
        let mut sorted_targets = ids.to_vec();
        sorted_targets.sort_unstable();
        if let ControlFlow::Break(Some(duplicate)) =
            sorted_targets.iter().try_fold(None, |last, current| {
                last.map_or_else(
                    || ControlFlow::Continue(Some(current)),
                    |last| {
                        if last == current {
                            ControlFlow::Break(Some(current))
                        } else {
                            ControlFlow::Continue(Some(current))
                        }
                    },
                )
            })
        {
            panic!("Duplicate qubit id '{}' found in application.", duplicate);
        }

        let locs: Vec<usize> = ids
            .iter()
            .map(|id| {
                *self
                    .id_map
                    .get(id)
                    .unwrap_or_else(|| panic!("Unable to find qubit with id {}", id))
            })
            .collect();

        self.check_joint_probability(&locs)
    }

    /// Measures the qubit with the given id, collapsing the state based on the measured result.
    /// # Panics
    ///
    /// This funciton will panic if the given identifier does not correspond to an allocated qubit.
    #[must_use]
    pub(crate) fn measure(&mut self, id: usize) -> bool {
        self.measure_impl(
            *self
                .id_map
                .get(&id)
                .unwrap_or_else(|| panic!("Unable to find qubit with id {}", id)),
        )
    }

    /// Utility that performs the actual measurement and collapse of the state for the given
    /// location.
    fn measure_impl(&mut self, loc: usize) -> bool {
        let mut rng = rand::thread_rng();
        let random_sample: f64 = rng.gen();
        let res = random_sample < self.check_joint_probability(&[loc]);
        self.collapse(loc, res);
        res
    }

    /// Performs a joint measurement to get the parity of the given qubits, collapsing the state
    /// based on the measured result.
    /// # Panics
    ///
    /// This function will panic if any of the given identifiers do not correspond to an allocated qubit.
    /// This function will panic if any of the given identifiers are duplicates.
    #[must_use]
    pub(crate) fn joint_measure(&mut self, ids: &[usize]) -> bool {
        let mut sorted_targets = ids.to_vec();
        sorted_targets.sort_unstable();
        if let ControlFlow::Break(Some(duplicate)) =
            sorted_targets.iter().try_fold(None, |last, current| {
                last.map_or_else(
                    || ControlFlow::Continue(Some(current)),
                    |last| {
                        if last == current {
                            ControlFlow::Break(Some(current))
                        } else {
                            ControlFlow::Continue(Some(current))
                        }
                    },
                )
            })
        {
            panic!("Duplicate qubit id '{}' found in application.", duplicate);
        }

        let locs: Vec<usize> = ids
            .iter()
            .map(|id| {
                *self
                    .id_map
                    .get(id)
                    .unwrap_or_else(|| panic!("Unable to find qubit with id {}", id))
            })
            .collect();

        let mut rng = rand::thread_rng();
        let random_sample: f64 = rng.gen();
        let res = random_sample < self.check_joint_probability(&locs);
        self.joint_collapse(&locs, res);
        res
    }

    /// Utility to get the sum of all probabilies where an odd number of the bits at the given locations
    /// are set. This corresponds to the probability of jointly measuring those qubits in the computational
    /// basis.
    fn check_joint_probability(&self, locs: &[usize]) -> f64 {
        let mask = locs.iter().fold(BigUint::zero(), |accum, loc| {
            accum | (BigUint::one() << loc)
        });
        self.state.iter().fold(0.0_f64, |accum, (index, val)| {
            if (index & &mask).count_ones() & 1 > 0 {
                accum + val.norm_sqr()
            } else {
                accum
            }
        })
    }

    /// Utility to collapse the probability at the given location based on the boolean value. This means
    /// that if the given value is 'true' then all keys in the sparse state where the given location
    /// has a zero bit will be reduced to zero and removed. Then the sparse state is normalized.
    fn collapse(&mut self, loc: usize, val: bool) {
        self.joint_collapse(&[loc], val);
    }

    /// Utility to collapse the joint probability of a particular set of locations in the sparse state.
    /// The entries that do not correspond to the given boolean value are removed, and then the whole
    /// state is normalized.
    fn joint_collapse(&mut self, locs: &[usize], val: bool) {
        let mask = locs.iter().fold(BigUint::zero(), |accum, loc| {
            accum | (BigUint::one() << loc)
        });

        let mut new_state = FxHashMap::default();
        let mut scaling_denominator = 0.0;
        for (k, v) in self.state.drain() {
            if ((&k & &mask).count_ones() & 1 > 0) == val {
                new_state.insert(k, v);
                scaling_denominator += v.norm_sqr();
            }
        }

        // Normalize the new state using the accumulated scaling.
        let scaling = 1.0 / scaling_denominator.sqrt();
        new_state.iter_mut().for_each(|(_, v)| *v *= scaling);

        self.state = new_state;
    }

    /// Swaps the mapped ids for the given qubits.
    pub(crate) fn swap_qubit_ids(&mut self, qubit1: usize, qubit2: usize) {
        let qubit1_mapped = *self
            .id_map
            .get(&qubit1)
            .unwrap_or_else(|| panic!("Unable to find qubit with id {}", qubit1));
        let qubit2_mapped = *self
            .id_map
            .get(&qubit2)
            .unwrap_or_else(|| panic!("Unable to find qubit with id {}", qubit2));
        *self.id_map.get_mut(&qubit1).unwrap() = qubit2_mapped;
        *self.id_map.get_mut(&qubit2).unwrap() = qubit1_mapped;
    }

    /// Swaps the states of two qubits throughout the sparse state map.
    pub(crate) fn swap_qubit_state(&mut self, qubit1: usize, qubit2: usize) {
        if qubit1 == qubit2 {
            return;
        }

        let (q1, q2) = (qubit1 as u64, qubit2 as u64);

        // Swap entries in the sparse state to correspond to swapping of two qubits' locations.
        self.state = self
            .state
            .drain()
            .fold(FxHashMap::default(), |mut accum, (k, v)| {
                if k.bit(q1) == k.bit(q2) {
                    accum.insert(k, v);
                } else {
                    let mut new_k = k.clone();
                    new_k.set_bit(q1, !k.bit(q1));
                    new_k.set_bit(q2, !k.bit(q2));
                    accum.insert(new_k, v);
                }
                accum
            });
    }

    /// Verifies that the given target and list of controls does not contain any duplicate entries, and returns
    /// those values mapped to internal identifiers and converted to `u64`.
    fn resolve_and_check_qubits(&self, target: usize, ctls: &[usize]) -> (u64, Vec<u64>) {
        let target = *self
            .id_map
            .get(&target)
            .unwrap_or_else(|| panic!("Unable to find qubit with id {}", target))
            as u64;

        let ctls: Vec<u64> = ctls
            .iter()
            .map(|c| {
                *self
                    .id_map
                    .get(c)
                    .unwrap_or_else(|| panic!("Unable to find qubit with id {}", c))
                    as u64
            })
            .collect();

        let mut sorted_qubits = ctls.clone();
        sorted_qubits.push(target);
        sorted_qubits.sort_unstable();
        if let ControlFlow::Break(Some(duplicate)) =
            sorted_qubits.iter().try_fold(None, |last, current| {
                last.map_or_else(
                    || ControlFlow::Continue(Some(current)),
                    |last| {
                        if last == current {
                            ControlFlow::Break(Some(current))
                        } else {
                            ControlFlow::Continue(Some(current))
                        }
                    },
                )
            })
        {
            panic!("Duplicate qubit id '{}' found in application.", duplicate);
        }

        (target, ctls)
    }

    /// Utility for performing an in-place update of the state vector with the given target and controls.
    /// Here, "in-place" indicates that the given transformation operation can calulate a new entry in the
    /// state vector using only one entry of the state vector as input and does not need to refer to any
    /// other entries. This covers the multicontrolled gates except for H, Rx, and Ry.
    fn controlled_gate<F>(&mut self, ctls: &[usize], target: usize, mut op: F)
    where
        F: FnMut((BigUint, Complex64), u64) -> (BigUint, Complex64),
    {
        let (target, ctls) = self.resolve_and_check_qubits(target, ctls);

        self.state = self.state.drain().into_iter().fold(
            SparseState::default(),
            |mut accum, (index, value)| {
                let (k, v) = if ctls.iter().all(|c| index.bit(*c as u64)) {
                    op((index, value), target as u64)
                } else {
                    (index, value)
                };
                if !v.is_nearly_zero() {
                    accum.insert(k, v);
                }
                accum
            },
        );
    }

    /// Performs the Pauli-X transformation on a single state.
    fn x_transform((mut index, val): (BigUint, Complex64), target: u64) -> (BigUint, Complex64) {
        index.set_bit(target, !index.bit(target));
        (index, val)
    }

    /// Single qubit X gate.
    pub(crate) fn x(&mut self, target: usize) {
        self.controlled_gate(&[], target, Self::x_transform);
    }

    /// Multi-controlled X gate.
    pub(crate) fn mcx(&mut self, ctls: &[usize], target: usize) {
        self.controlled_gate(ctls, target, Self::x_transform);
    }

    /// Performs the Pauli-Y transformation on a single state.
    fn y_transform(
        (mut index, mut val): (BigUint, Complex64),
        target: u64,
    ) -> (BigUint, Complex64) {
        index.set_bit(target, !index.bit(target));
        val *= if index.bit(target) {
            Complex64::i()
        } else {
            -Complex64::i()
        };
        (index, val)
    }

    /// Single qubit Y gate.
    pub(crate) fn y(&mut self, target: usize) {
        self.controlled_gate(&[], target, Self::y_transform);
    }

    /// Multi-controlled Y gate.
    pub(crate) fn mcy(&mut self, ctls: &[usize], target: usize) {
        self.controlled_gate(ctls, target, Self::y_transform);
    }

    /// Performs a phase transformation (a rotation in the computational basis) on a single state.
    fn phase_transform(
        phase: Complex64,
        (index, val): (BigUint, Complex64),
        target: u64,
    ) -> (BigUint, Complex64) {
        let val = val
            * if index.bit(target) {
                phase
            } else {
                Complex64::one()
            };
        (index, val)
    }

    /// Multi-controlled phase rotation ("G" gate).
    pub(crate) fn mcphase(&mut self, ctls: &[usize], phase: Complex64, target: usize) {
        self.controlled_gate(ctls, target, |(index, val), target| {
            Self::phase_transform(phase, (index, val), target)
        });
    }

    /// Performs the Pauli-Z transformation on a single state.
    fn z_transform((index, val): (BigUint, Complex64), target: u64) -> (BigUint, Complex64) {
        Self::phase_transform(-Complex64::one(), (index, val), target)
    }

    /// Single qubit Z gate.
    pub(crate) fn z(&mut self, target: usize) {
        self.controlled_gate(&[], target, Self::z_transform);
    }

    /// Multi-controlled Z gate.
    pub(crate) fn mcz(&mut self, ctls: &[usize], target: usize) {
        self.controlled_gate(ctls, target, Self::z_transform);
    }

    /// Performs the S transformation on a single state.
    fn s_transform((index, val): (BigUint, Complex64), target: u64) -> (BigUint, Complex64) {
        Self::phase_transform(Complex64::i(), (index, val), target)
    }

    /// Single qubit S gate.
    pub(crate) fn s(&mut self, target: usize) {
        self.controlled_gate(&[], target, Self::s_transform);
    }

    /// Multi-controlled S gate.
    pub(crate) fn mcs(&mut self, ctls: &[usize], target: usize) {
        self.controlled_gate(ctls, target, Self::s_transform);
    }

    /// Performs the adjoint S transformation on a signle state.
    fn sadj_transform((index, val): (BigUint, Complex64), target: u64) -> (BigUint, Complex64) {
        Self::phase_transform(-Complex64::i(), (index, val), target)
    }

    /// Single qubit Adjoint S Gate.
    pub(crate) fn sadj(&mut self, target: usize) {
        self.controlled_gate(&[], target, Self::sadj_transform);
    }

    /// Multi-controlled Adjoint S gate.
    pub(crate) fn mcsadj(&mut self, ctls: &[usize], target: usize) {
        self.controlled_gate(ctls, target, Self::sadj_transform);
    }

    /// Performs the T transformation on a single state.
    fn t_transform((index, val): (BigUint, Complex64), target: u64) -> (BigUint, Complex64) {
        Self::phase_transform(
            Complex64::new(FRAC_1_SQRT_2, FRAC_1_SQRT_2),
            (index, val),
            target,
        )
    }

    /// Single qubit T gate.
    pub(crate) fn t(&mut self, target: usize) {
        self.controlled_gate(&[], target, Self::t_transform);
    }

    /// Multi-controlled T gate.
    pub(crate) fn mct(&mut self, ctls: &[usize], target: usize) {
        self.controlled_gate(ctls, target, Self::t_transform);
    }

    /// Performs the adjoint T transformation to a single state.
    fn tadj_transform((index, val): (BigUint, Complex64), target: u64) -> (BigUint, Complex64) {
        Self::phase_transform(
            Complex64::new(FRAC_1_SQRT_2, -FRAC_1_SQRT_2),
            (index, val),
            target,
        )
    }

    /// Single qubit Adjoint T gate.
    pub(crate) fn tadj(&mut self, target: usize) {
        self.controlled_gate(&[], target, Self::tadj_transform);
    }

    /// Multi-controlled Adjoint T gate.
    pub(crate) fn mctadj(&mut self, ctls: &[usize], target: usize) {
        self.controlled_gate(ctls, target, Self::tadj_transform);
    }

    /// Performs the Rz transformation with the given angle to a single state.
    fn rz_transform(
        (index, val): (BigUint, Complex64),
        theta: f64,
        target: u64,
    ) -> (BigUint, Complex64) {
        let val = val
            * Complex64::exp(Complex64::new(
                0.0,
                theta / 2.0 * if index.bit(target) { 1.0 } else { -1.0 },
            ));
        (index, val)
    }

    /// Single qubit Rz gate.
    pub(crate) fn rz(&mut self, theta: f64, target: usize) {
        self.controlled_gate(&[], target, |(index, val), target| {
            Self::rz_transform((index, val), theta, target)
        });
    }

    /// Multi-controlled Rz gate.
    pub(crate) fn mcrz(&mut self, ctls: &[usize], theta: f64, target: usize) {
        self.controlled_gate(ctls, target, |(index, val), target| {
            Self::rz_transform((index, val), theta, target)
        });
    }

    /// Single qubit H gate.
    pub(crate) fn h(&mut self, target: usize) {
        self.mch(&[], target);
    }

    /// Multi-controlled H gate.
    pub(crate) fn mch(&mut self, ctls: &[usize], target: usize) {
        let (target, ctls) = self.resolve_and_check_qubits(target, ctls);

        // This operation cannot be done in-place so create a new empty state vector to populate.
        let mut new_state = SparseState::default();

        let mut flipped = BigUint::zero();
        flipped.set_bit(target, true);

        for (index, value) in &self.state {
            if ctls.iter().all(|c| index.bit(*c)) {
                let flipped_index = index ^ &flipped;
                if !self.state.contains_key(&flipped_index) {
                    // The state vector does not have an entry for the state where the target is flipped
                    // and all other qubits are the same, meaning there is no superposition for this state.
                    // Create the additional state caluclating the resulting superposition.
                    let mut zero_bit_index = index.clone();
                    zero_bit_index.set_bit(target, false);
                    new_state.insert(zero_bit_index, value * std::f64::consts::FRAC_1_SQRT_2);

                    let mut one_bit_index = index.clone();
                    one_bit_index.set_bit(target, true);
                    new_state.insert(
                        one_bit_index,
                        value
                            * std::f64::consts::FRAC_1_SQRT_2
                            * (if index.bit(target) { -1.0 } else { 1.0 }),
                    );
                } else if !index.bit(target) {
                    // The state vector already has a superposition for this state, so calculate the resulting
                    // updates using the value from the flipped state. Note we only want to perform this for one
                    // of the states to avoid duplication, so we pick the Zero state by checking the target bit
                    // in the index is not set.
                    let flipped_value = &self.state[&flipped_index];

                    let new_val = (value + flipped_value) as Complex64;
                    if !new_val.is_nearly_zero() {
                        new_state.insert(index.clone(), new_val * std::f64::consts::FRAC_1_SQRT_2);
                    }

                    let new_val = (value - flipped_value) as Complex64;
                    if !new_val.is_nearly_zero() {
                        new_state
                            .insert(index | &flipped, new_val * std::f64::consts::FRAC_1_SQRT_2);
                    }
                }
            } else {
                new_state.insert(index.clone(), *value);
            }
        }

        self.state = new_state;
    }

    /// Performs a rotation in the non-computational basis, which cannot be done in-place. This
    /// corresponds to an Rx or Ry depending on the requested sign flip.
    fn mcrotation(&mut self, ctls: &[usize], theta: f64, target: usize, sign_flip: bool) {
        // Calculate the matrix entries for the rotation by the given angle, respecting the sign flip.
        let m00 = Complex64::new(f64::cos(theta / 2.0), 0.0);
        let m01 = Complex64::new(0.0, f64::sin(theta / -2.0))
            * if sign_flip {
                -Complex64::i()
            } else {
                Complex64::one()
            };

        if m00.is_nearly_zero() {
            // This is just a Pauli rotation.
            if sign_flip {
                self.mcy(ctls, target);
            } else {
                self.mcx(ctls, target);
            }
        } else if m01.is_nearly_zero() {
            // This is just identity, so we can no-op.
        } else {
            let (target, ctls) = self.resolve_and_check_qubits(target, ctls);
            let mut new_state = SparseState::default();
            let m10 = m01 * if sign_flip { -1.0 } else { 1.0 };
            let mut flipped = BigUint::zero();
            flipped.set_bit(target, true);

            for (index, value) in &self.state {
                if ctls.iter().all(|c| index.bit(*c)) {
                    let flipped_index = index ^ &flipped;
                    if !self.state.contains_key(&flipped_index) {
                        // The state vector doesn't have an entry for the flipped target bit, so there
                        // isn't a superposition. Calculate the superposition using the matrix entries.
                        if index.bit(target) {
                            new_state.insert(flipped_index, value * m01);
                            new_state.insert(index.clone(), value * m00);
                        } else {
                            new_state.insert(index.clone(), value * m00);
                            new_state.insert(flipped_index, value * m10);
                        }
                    } else if !index.bit(target) {
                        // There is already a superposition of the target for this state, so calculate the new
                        // entries using the values from the flipped state. Note we only want to do this for one of
                        // the states, so we pick the Zero state by checking the target bit in the index is not set.
                        let flipped_val = self.state[&flipped_index];

                        let new_val = (value * m00 + flipped_val * m01) as Complex64;
                        if !new_val.is_nearly_zero() {
                            new_state.insert(index.clone(), new_val);
                        }

                        let new_val = (value * m10 + flipped_val * m00) as Complex64;
                        if !new_val.is_nearly_zero() {
                            new_state.insert(flipped_index, new_val);
                        }
                    }
                } else {
                    new_state.insert(index.clone(), *value);
                }
            }

            self.state = new_state;
        }
    }

    /// Single qubit Rx gate.
    pub(crate) fn rx(&mut self, theta: f64, target: usize) {
        self.mcrotation(&[], theta, target, false);
    }

    /// Multi-controlled Rx gate.
    pub(crate) fn mcrx(&mut self, ctls: &[usize], theta: f64, target: usize) {
        self.mcrotation(ctls, theta, target, false);
    }

    /// Single qubit Ry gate.
    pub(crate) fn ry(&mut self, theta: f64, target: usize) {
        self.mcrotation(&[], theta, target, true);
    }

    /// Multi-controlled Ry gate.
    pub(crate) fn mcry(&mut self, ctls: &[usize], theta: f64, target: usize) {
        self.mcrotation(ctls, theta, target, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn almost_equal(a: f64, b: f64) -> bool {
        a.max(b) - b.min(a) <= 1e-10
    }

    // Test that basic allocation and release of qubits doesn't fail.
    #[test]
    fn test_alloc_release() {
        let sim = &mut QuantumSim::default();
        for i in 0..16 {
            assert_eq!(sim.allocate(), i);
        }
        sim.release(4);
        sim.release(7);
        sim.release(12);
        assert_eq!(sim.allocate(), 4);
        for i in 0..7 {
            sim.release(i);
        }
        for i in 8..12 {
            sim.release(i);
        }
        for i in 13..16 {
            sim.release(i);
        }
    }

    /// Verifies that application of gates to a qubit results in the correct probabilities.
    #[test]
    fn test_probability() {
        let mut sim = QuantumSim::default();
        let q = sim.allocate();
        let extra = sim.allocate();
        assert!(almost_equal(0.0, sim.joint_probability(&[q])));
        sim.x(q);
        assert!(almost_equal(1.0, sim.joint_probability(&[q])));
        sim.x(q);
        assert!(almost_equal(0.0, sim.joint_probability(&[q])));
        sim.h(q);
        assert!(almost_equal(0.5, sim.joint_probability(&[q])));
        sim.h(q);
        assert!(almost_equal(0.0, sim.joint_probability(&[q])));
        sim.x(q);
        sim.h(q);
        sim.s(q);
        assert!(almost_equal(0.5, sim.joint_probability(&[q])));
        sim.sadj(q);
        sim.h(q);
        sim.x(q);
        assert!(almost_equal(0.0, sim.joint_probability(&[q])));
        sim.release(extra);
        sim.release(q);
    }

    /// Verify that a qubit in superposition has probability corresponding the measured value and
    /// can be operationally reset back into the ground state.
    #[test]
    fn test_measure() {
        let mut sim = QuantumSim::default();
        let q = sim.allocate();
        let extra = sim.allocate();
        assert!(!sim.measure(q));
        sim.x(q);
        assert!(sim.measure(q));
        let mut res = false;
        while !res {
            sim.h(q);
            res = sim.measure(q);
            assert!(almost_equal(
                sim.joint_probability(&[q]),
                if res { 1.0 } else { 0.0 }
            ));
            if res {
                sim.x(q);
            }
        }
        assert!(almost_equal(sim.joint_probability(&[q]), 0.0));
        sim.release(extra);
        sim.release(q);
    }

    /// Verify joint probability works as expected, namely that it corresponds to the parity of the
    /// qubits.
    #[test]
    fn test_joint_probability() {
        let mut sim = QuantumSim::default();
        let q0 = sim.allocate();
        let q1 = sim.allocate();
        assert!(almost_equal(0.0, sim.joint_probability(&[q0, q1])));
        sim.x(q0);
        assert!(almost_equal(1.0, sim.joint_probability(&[q0, q1])));
        sim.x(q1);
        assert!(almost_equal(0.0, sim.joint_probability(&[q0, q1])));
        assert!(almost_equal(1.0, sim.joint_probability(&[q0])));
        assert!(almost_equal(1.0, sim.joint_probability(&[q1])));
        sim.h(q0);
        assert!(almost_equal(0.5, sim.joint_probability(&[q0, q1])));
        sim.release(q1);
        sim.release(q0);
    }

    /// Verify joint measurement works as expected, namely that it corresponds to the parity of the
    /// qubits.
    #[test]
    fn test_joint_measurement() {
        let mut sim = QuantumSim::default();
        let q0 = sim.allocate();
        let q1 = sim.allocate();
        assert!(!sim.joint_measure(&[q0, q1]));
        sim.x(q0);
        assert!(sim.joint_measure(&[q0, q1]));
        sim.x(q1);
        assert!(!sim.joint_measure(&[q0, q1]));
        assert!(sim.joint_measure(&[q0]));
        assert!(sim.joint_measure(&[q1]));
        sim.h(q0);
        let res = sim.joint_measure(&[q0, q1]);
        assert!(almost_equal(
            if res { 1.0 } else { 0.0 },
            sim.joint_probability(&[q0, q1])
        ));
        sim.release(q1);
        sim.release(q0);
    }

    /// Test arbitrary controls, which should extend the applied unitary matrix.
    #[test]
    fn test_arbitrary_controls() {
        let mut sim = QuantumSim::default();
        let q0 = sim.allocate();
        let q1 = sim.allocate();
        let q2 = sim.allocate();
        assert!(almost_equal(0.0, sim.joint_probability(&[q0])));
        sim.h(q0);
        assert!(almost_equal(0.5, sim.joint_probability(&[q0])));
        sim.h(q0);
        assert!(almost_equal(0.0, sim.joint_probability(&[q0])));
        sim.mch(&[q1], q0);
        assert!(almost_equal(0.0, sim.joint_probability(&[q0])));
        sim.x(q1);
        sim.mch(&[q1], q0);
        assert!(almost_equal(0.5, sim.joint_probability(&[q0])));
        sim.mch(&[q2, q1], q0);
        assert!(almost_equal(0.5, sim.joint_probability(&[q0])));
        sim.x(q2);
        sim.mch(&[q2, q1], q0);
        assert!(almost_equal(0.0, sim.joint_probability(&[q0])));
        sim.x(q0);
        sim.x(q1);
        sim.release(q2);
        sim.release(q1);
        sim.release(q0);
    }

    /// Verify that targets cannot be duplicated.
    #[test]
    #[should_panic(expected = "Duplicate qubit id '0' found in application.")]
    fn test_duplicate_target() {
        let mut sim = QuantumSim::new();
        let q = sim.allocate();
        sim.mcx(&[q], q);
    }

    /// Verify that controls cannot be duplicated.
    #[test]
    #[should_panic(expected = "Duplicate qubit id '1' found in application.")]
    fn test_duplicate_control() {
        let mut sim = QuantumSim::new();
        let q = sim.allocate();
        let c = sim.allocate();
        sim.mcx(&[c, c], q);
    }

    /// Verify that targets aren't in controls.
    #[test]
    #[should_panic(expected = "Duplicate qubit id '0' found in application.")]
    fn test_target_in_control() {
        let mut sim = QuantumSim::new();
        let q = sim.allocate();
        let c = sim.allocate();
        sim.mcx(&[c, q], q);
    }

    /// Large, entangled state handling.
    #[test]
    fn test_large_state() {
        let mut sim = QuantumSim::new();
        let ctl = sim.allocate();
        sim.h(ctl);
        for _ in 0..4999 {
            let q = sim.allocate();
            sim.mcx(&[ctl], q);
        }
        let _ = sim.measure(ctl);
        for i in 0..5000 {
            sim.release(i);
        }
    }

    /// Utility for testing operation equivalence.
    fn assert_operation_equal_referenced<F1, F2>(mut op: F1, mut reference: F2, count: usize)
    where
        F1: FnMut(&mut QuantumSim, &[usize]),
        F2: FnMut(&mut QuantumSim, &[usize]),
    {
        let mut sim = QuantumSim::default();

        // Allocte the control we use to verify behavior.
        let ctl = sim.allocate();
        sim.h(ctl);

        // Allocate the requested number of targets, entangling the control with them.
        let mut qs = vec![];
        for _ in 0..count {
            let q = sim.allocate();
            sim.mcx(&[ctl], q);
            qs.push(q);
        }

        op(&mut sim, &qs);
        reference(&mut sim, &qs);

        // Undo the entanglement.
        for q in qs {
            sim.mcx(&[ctl], q);
        }
        sim.h(ctl);

        // We know the operations are equal if the control is left in the zero state.
        assert!(sim.joint_probability(&[ctl]).is_nearly_zero());

        // Sparse state vector should have one entry for |0⟩.
        assert_eq!(sim.state.len(), 1);
    }

    #[test]
    fn test_h() {
        assert_operation_equal_referenced(
            |sim, qs| {
                sim.h(qs[0]);
            },
            |sim, qs| {
                sim.h(qs[0]);
            },
            1,
        );
    }

    #[test]
    fn test_x() {
        assert_operation_equal_referenced(
            |sim, qs| {
                sim.x(qs[0]);
            },
            |sim, qs| {
                sim.x(qs[0]);
            },
            1,
        );
    }

    #[test]
    fn test_y() {
        assert_operation_equal_referenced(
            |sim, qs| {
                sim.y(qs[0]);
            },
            |sim, qs| {
                sim.y(qs[0]);
            },
            1,
        );
    }

    #[test]
    fn test_z() {
        assert_operation_equal_referenced(
            |sim, qs| {
                sim.z(qs[0]);
            },
            |sim, qs| {
                sim.z(qs[0]);
            },
            1,
        );
    }

    #[test]
    fn test_s() {
        assert_operation_equal_referenced(
            |sim, qs| {
                sim.s(qs[0]);
            },
            |sim, qs| {
                sim.sadj(qs[0]);
            },
            1,
        );
    }

    #[test]
    fn test_sadj() {
        assert_operation_equal_referenced(
            |sim, qs| {
                sim.sadj(qs[0]);
            },
            |sim, qs| {
                sim.s(qs[0]);
            },
            1,
        );
    }

    #[test]
    fn test_cx() {
        assert_operation_equal_referenced(
            |sim, qs| {
                sim.mcx(&[qs[0]], qs[1]);
            },
            |sim, qs| {
                sim.mcx(&[qs[0]], qs[1]);
            },
            2,
        );
    }

    #[test]
    fn test_cz() {
        assert_operation_equal_referenced(
            |sim, qs| {
                sim.mcz(&[qs[0]], qs[1]);
            },
            |sim, qs| {
                sim.mcz(&[qs[0]], qs[1]);
            },
            2,
        );
    }

    #[test]
    fn test_swap() {
        assert_operation_equal_referenced(
            |sim, qs| {
                sim.swap_qubit_ids(qs[0], qs[1]);
            },
            |sim, qs| {
                sim.swap_qubit_ids(qs[0], qs[1]);
            },
            2,
        );
    }

    #[test]
    fn test_rz() {
        assert_operation_equal_referenced(
            |sim, qs| {
                sim.rz(PI / 7.0, qs[0]);
            },
            |sim, qs| {
                sim.rz(-PI / 7.0, qs[0]);
            },
            1,
        );
    }

    #[test]
    fn test_rx() {
        assert_operation_equal_referenced(
            |sim, qs| {
                sim.rx(PI / 7.0, qs[0]);
            },
            |sim, qs| {
                sim.rx(-PI / 7.0, qs[0]);
            },
            1,
        );
    }

    #[test]
    fn test_ry() {
        assert_operation_equal_referenced(
            |sim, qs| {
                sim.ry(PI / 7.0, qs[0]);
            },
            |sim, qs| {
                sim.ry(-PI / 7.0, qs[0]);
            },
            1,
        );
    }

    #[test]
    fn test_mcri() {
        assert_operation_equal_referenced(
            |sim, qs| {
                sim.mcphase(
                    &qs[2..3],
                    Complex64::exp(Complex64::new(0.0, -(PI / 7.0) / 2.0)),
                    qs[1],
                );
            },
            |sim, qs| {
                sim.mcphase(
                    &qs[2..3],
                    Complex64::exp(Complex64::new(0.0, (PI / 7.0) / 2.0)),
                    qs[1],
                );
            },
            3,
        );
    }
}