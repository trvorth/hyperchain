#[cfg(feature = "zk")]
use bellman::{Circuit, ConstraintSystem, LinearCombination, SynthesisError};
#[cfg(feature = "zk")]
use bls12_381::Scalar;

#[cfg(feature = "zk")]
use sha2::{Digest, Sha256};
#[cfg(feature = "zk")]
use std::io;

#[cfg(feature = "zk")]
#[derive(Debug)]
pub enum ZKError {
    Synthesis(SynthesisError),
    Io(io::Error),
    ProofGeneration(SynthesisError),
    Verification(SynthesisError),
    InvalidInputLength,
}

#[cfg(feature = "zk")]
impl From<SynthesisError> for ZKError {
    fn from(err: SynthesisError) -> Self {
        ZKError::Synthesis(err)
    }
}

#[cfg(feature = "zk")]
impl From<io::Error> for ZKError {
    fn from(err: io::Error) -> Self {
        ZKError::Io(err)
    }
}

#[cfg(feature = "zk")]
#[derive(Clone)]
pub struct UtxoCircuit {
    pub amount: Option<u64>,
    pub address: Option<[u8; 32]>,
}

#[cfg(feature = "zk")]
impl Circuit<Scalar> for UtxoCircuit {
    fn synthesize<CS: ConstraintSystem<Scalar>>(self, cs: &mut CS) -> Result<(), SynthesisError> {
        let amount_var = cs.alloc(
            || "amount",
            || {
                self.amount
                    .map(Scalar::from)
                    .ok_or(SynthesisError::AssignmentMissing)
            },
        )?;

        let mut amount_bits_vars = Vec::with_capacity(64);
        for i in 0..64 {
            let bit_val = self.amount.map(|a| (a >> i) & 1);
            let bit_var = cs.alloc(
                || format!("amount bit {}", i),
                || {
                    bit_val
                        .map(Scalar::from)
                        .ok_or(SynthesisError::AssignmentMissing)
                },
            )?;
            amount_bits_vars.push(bit_var);
        }

        let mut computed_amount = LinearCombination::<Scalar>::zero();
        let mut coeff = Scalar::one();
        for (i, bit_var) in amount_bits_vars.iter().enumerate() {
            cs.enforce(
                || format!("bit {} enforcement", i),
                |lc| lc + *bit_var,
                |lc| lc + CS::one() - *bit_var,
                |lc| lc,
            );
            computed_amount = computed_amount + (coeff, *bit_var);
            coeff = coeff.double();
        }

        cs.enforce(
            || "amount reconstruction",
            |_| computed_amount,
            |lc| lc + CS::one(),
            |lc| lc + amount_var,
        );

        let _address_hash_var = cs.alloc_input(
            || "address_hash",
            || {
                let address_hash = self
                    .address
                    .map(|addr| Sha256::digest(addr))
                    .ok_or(SynthesisError::AssignmentMissing)?;
                let address_hash_slice: [u8; 32] = address_hash.into();
                Scalar::from_bytes(&address_hash_slice)
                    .into_option()
                    .ok_or(SynthesisError::AssignmentMissing)
            },
        )?;

        Ok(())
    }
}
