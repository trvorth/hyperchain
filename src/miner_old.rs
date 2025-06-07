use bellman::{Circuit, ConstraintSystem, SynthesisError};
use bls12_381::{Bls12, Scalar};
use bellman::groth16::{self, PreparedVerifyingKey, Proof, Parameters};
use rand::rngs::OsRng;
use subtle::CtOption;
use thiserror::Error;
use std::io::Read;

/// Errors for zero-knowledge operations.
#[derive(Error, Debug)]
pub enum ZKError {
    #[error("Synthesis error: {0}")]
    Synthesis(#[from] SynthesisError),
    #[error("Proof generation failed: {0}")]
    ProofGeneration(String),
    #[error("Verification failed")]
    Verification,
}

/// Circuit for UTXO validation.
#[derive(Clone)]
pub struct UtxoCircuit {
    pub amount: Option<u64>,
    pub address: Option<[u8; 32]>,
}

impl Circuit<Scalar> for UtxoCircuit {
    fn synthesize<CS: ConstraintSystem<Scalar>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        let amount = cs.alloc(
            || "amount",
            || self.amount.map(Scalar::from).ok_or(SynthesisError::AssignmentMissing),
        )?;
        let _address = cs.alloc(
            || "address",
            || {
                self.address
                    .and_then(|a| Scalar::from_bytes(&a).into())
                    .ok_or(SynthesisError::AssignmentMissing)
            },
        )?;
        cs.enforce(
            || "amount constraint",
            |lc| lc + amount,
            |lc| lc + CS::one(),
            |lc| lc + amount,
        );
        Ok(())
    }
}

/// Zero-knowledge proof structure.
#[derive(Clone)]
pub struct ZKProof {
    proof: Proof<Bls12>,
    public_inputs: Vec<Scalar>,
}

impl ZKProof {
    /// Creates a new zero-knowledge proof.
    pub fn new(amount: u64, address: [u8; 32]) -> Result<Self, ZKError> {
        let circuit = UtxoCircuit {
            amount: Some(amount),
            address: Some(address),
        };
        let params = groth16::generate_random_parameters::<Bls12, _, _>(circuit.clone(), &mut OsRng)
            .map_err(|e| ZKError::ProofGeneration(e.to_string()))?;
        let proof = groth16::create_random_proof(circuit, &params, &mut OsRng)
            .map_err(|e| ZKError::ProofGeneration(e.to_string()))?;
        let public_inputs = vec![Scalar::from(amount)];
        Ok(ZKProof { proof, public_inputs })
    }

    /// Verifies the proof.
    pub fn verify(&self, verifying_key: &PreparedVerifyingKey<Bls12>) -> Result<(), ZKError> {
        groth16::verify_proof(verifying_key, &self.proof, &self.public_inputs)
            .map_err(|_| ZKError::Verification)?;
        Ok(())
    }

    /// Generates circuit parameters.
    pub fn generate_parameters() -> Result<Parameters<Bls12>, ZKError> {
        let circuit = UtxoCircuit {
            amount: None,
            address: None,
        };
        groth16::generate_random_parameters::<Bls12, _, _>(circuit, &mut OsRng)
            .map_err(|e| ZKError::ProofGeneration(e.to_string()))
    }

    /// Prepares the verifying key.
    pub fn prepare_verifying_key(params: &Parameters<Bls12>) -> PreparedVerifyingKey<Bls12> {
        groth16::prepare_verifying_key(&params.vk)
    }

    /// Serializes the proof to bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = vec![];
        self.proof.write(&mut bytes).unwrap();
        for input in &self.public_inputs {
            bytes.extend_from_slice(&input.to_bytes());
        }
        bytes
    }

    /// Deserializes a proof from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ZKError> {
        let mut cursor = std::io::Cursor::new(bytes);
        let proof = Proof::read(&mut cursor).map_err(|e| ZKError::ProofGeneration(e.to_string()))?;
        let mut public_inputs = vec![];
        while cursor.position() < bytes.len() as u64 {
            let mut chunk = [0u8; 32];
            cursor.read_exact(&mut chunk).map_err(|e| ZKError::ProofGeneration(e.to_string()))?;
            let scalar_opt = Scalar::from_bytes(&chunk);
            if bool::from(scalar_opt.is_some()) {
                public_inputs.push(scalar_opt.unwrap());
            }
        }
        Ok(ZKProof { proof, public_inputs })
    }
}