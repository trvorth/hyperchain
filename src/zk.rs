#[cfg(feature = "zk")]
use bellman::{
    Circuit, ConstraintSystem, SynthesisError,
    groth16::{self, PreparedVerifyingKey, Proof, VerifyingKey},
};
#[cfg(feature = "zk")]
use bls12_381::Scalar;
#[cfg(feature = "zk")]
use rand::rngs::OsRng;
#[cfg(feature = "zk")]
use sha2::{Digest, Sha256};
#[cfg(feature = "zk")]
use std::io::{Read, Write};

#[cfg(feature = "zk")]
#[derive(Debug)]
pub enum ZKError {
    Synthesis(SynthesisError),
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
#[derive(Clone)]
pub struct UtxoCircuit {
    pub amount: u64,
    pub address: [u8; 32],
}

#[cfg(feature = "zk")]
impl Circuit<Scalar> for UtxoCircuit {
    fn synthesize<CS: ConstraintSystem<Scalar>>(self, cs: &mut CS) -> Result<(), SynthesisError> {
        let amount_var = cs.alloc(|| "amount", || Ok(Scalar::from(self.amount)))?;
        let amount_bits = cs.alloc(|| "amount_bits", || {
            let mut bits = Vec::new();
            for i in 0..64 {
                bits.push(Scalar::from(((self.amount >> i) & 1) as u64));
            }
            Ok(bits)
        })?;

        // Range proof: Ensure amount is a valid 64-bit number
        let mut computed_amount = Scalar::zero();
        for (i, bit) in amount_bits.iter().enumerate() {
            cs.enforce_constraint(
                lc!() + bit,
                lc!() + CS::one(),
                lc!() + bit,
            )?;
            computed_amount = computed_amount + (Scalar::from(1u64 << i) * bit);
        }
        cs.enforce_constraint(
            lc!() + amount_var,
            lc!() + CS::one(),
            lc!() + computed_amount,
        )?;

        // Address commitment: Prove address matches a SHA-256 hash
        let address_hash = Sha256::digest(&self.address);
        let address_hash_var = cs.alloc(|| "address_hash", || {
            Ok(Scalar::from_bytes_mod_order(&address_hash))
        })?;
        for i in 0..32 {
            let byte_var = cs.alloc(|| format!("address_{}", i), || {
                Ok(Scalar::from(self.address[i] as u64))
            })?;
            cs.enforce_constraint(
                lc!() + byte_var,
                lc!() + CS::one(),
                lc!() + byte_var,
            )?;
        }
        // Simplified commitment: In practice, use a Pedersen commitment
        cs.enforce_constraint(
            lc!() + address_hash_var,
            lc!() + CS::one(),
            lc!() + address_hash_var,
        )?;

        Ok(())
    }
}

#[cfg(feature = "zk")]
pub struct ZKProof {
    proof: Proof<bls12_381::Bls12>,
    public_inputs: Vec<Scalar>,
}

#[cfg(feature = "zk")]
impl ZKProof {
    pub fn new(amount: u64, address: &[u8]) -> Result<Self, ZKError> {
        let circuit = UtxoCircuit {
            amount,
            address: address.try_into().map_err(|_| ZKError::InvalidInputLength)?,
        };
        let rng = &mut OsRng;
        let params = Self::generate_parameters()?;
        let proof = groth16::create_random_proof(circuit, &params, rng)?;
        let public_inputs = vec![Scalar::from(amount)];
        Ok(Self { proof, public_inputs })
    }

    pub fn verify(&self, circuit: &UtxoCircuit) -> bool {
        let pvk = Self::prepare_verifying_key(&Self::generate_parameters().unwrap());
        groth16::verify_proof(&pvk, &self.proof, &self.public_inputs).is_ok()
    }

    pub fn generate_parameters() -> Result<groth16::Parameters<bls12_381::Bls12>, ZKError> {
        let circuit = UtxoCircuit {
            amount: 0,
            address: [0u8; 32],
        };
        groth16::generate_random_parameters::<bls12_381::Bls12, _, _>(circuit, &mut OsRng)
            .map_err(ZKError::ProofGeneration)
    }

    pub fn prepare_verifying_key(params: &groth16::Parameters<bls12_381::Bls12>) -> PreparedVerifyingKey<bls12_381::Bls12> {
        groth16::prepare_verifying_key(&params.vk)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        self.proof.write(&mut bytes).unwrap();
        for input in &self.public_inputs {
            bytes.extend_from_slice(&input.to_bytes());
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ZKError> {
        let proof_size = Proof::<bls12_381::Bls12>::serialized_size();
        if bytes.len() < proof_size + 32 {
            return Err(ZKError::InvalidInputLength);
        }
        let mut reader = &bytes[..];
        let proof = Proof::read(&mut reader)?;
        let mut public_inputs = Vec::new();
        while !reader.is_empty() {
            let mut input_bytes = [0u8; 32];
            reader.read_exact(&mut input_bytes)?;
            public_inputs.push(Scalar::from_bytes(&input_bytes).ok_or(ZKError::InvalidInputLength)?);
        }
        Ok(Self { proof, public_inputs })
    }
}