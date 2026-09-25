//! BPF Loader Upgradeable (loader-v3): the Upgrade instruction and the three
//! account states a program upgrade touches.
//!
//! The instruction enum, the ProgramData derivation and the state sizes come
//! from the official `solana-loader-v3-interface`, pinned at the version the
//! execution runtime already resolves. Nothing here says whether an upgrade is
//! wanted, only what the bytes are.

use anyhow::{bail, ensure, Result};
use solana_address::Address;
use solana_loader_v3_interface::{
    instruction::UpgradeableLoaderInstruction, state::UpgradeableLoaderState,
};

pub fn id() -> Address {
    solana_sdk_ids::bpf_loader_upgradeable::ID
}

pub fn rent_sysvar() -> Address {
    solana_sdk_ids::sysvar::rent::ID
}

pub fn clock_sysvar() -> Address {
    solana_sdk_ids::sysvar::clock::ID
}

/// `[program]` under the loader: where a program's bytes live.
pub fn programdata_address(program: &Address) -> Address {
    solana_loader_v3_interface::get_program_data_address(program)
}

/// Position of each account in `Upgrade`, per the interface's own builder.
pub const UPGRADE_PROGRAMDATA: usize = 0;
pub const UPGRADE_PROGRAM: usize = 1;
pub const UPGRADE_BUFFER: usize = 2;
pub const UPGRADE_SPILL: usize = 3;
pub const UPGRADE_RENT: usize = 4;
pub const UPGRADE_CLOCK: usize = 5;
pub const UPGRADE_AUTHORITY: usize = 6;
pub const UPGRADE_ACCOUNTS: usize = 7;

/// What an instruction's data decodes to under the loader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoaderInstruction {
    /// `Upgrade`, in a form whose meaning does not depend on runtime features:
    /// the legacy four-byte encoding, or the explicit SIMD-0430 byte set to
    /// close the buffer. Both close the buffer on every runtime.
    Upgrade,
    /// `Upgrade { close_buffer: false }`. Before SIMD-0430 the runtime ignores
    /// the trailing byte and closes the buffer anyway; after it, it does not.
    /// What it does therefore depends on the cluster's features, not on the
    /// message, and a bounded binding does not accept that.
    UpgradeKeepingBuffer,
    /// Any other loader instruction, by name.
    Other(&'static str),
    Undecodable,
}

pub fn decode_instruction(data: &[u8]) -> LoaderInstruction {
    let Ok(decoded) = wincode::deserialize::<UpgradeableLoaderInstruction>(data) else {
        return LoaderInstruction::Undecodable;
    };
    match decoded {
        // Exactly the encodings the interface itself produces, and no trailing
        // bytes a later runtime might read.
        UpgradeableLoaderInstruction::Upgrade { close_buffer } => match (data, close_buffer) {
            ([3, 0, 0, 0], true) | ([3, 0, 0, 0, 1], true) => LoaderInstruction::Upgrade,
            ([3, 0, 0, 0, 0], false) => LoaderInstruction::UpgradeKeepingBuffer,
            _ => LoaderInstruction::Undecodable,
        },
        other => LoaderInstruction::Other(match other {
            UpgradeableLoaderInstruction::InitializeBuffer => "InitializeBuffer",
            UpgradeableLoaderInstruction::Write { .. } => "Write",
            UpgradeableLoaderInstruction::DeployWithMaxDataLen { .. } => "DeployWithMaxDataLen",
            UpgradeableLoaderInstruction::SetAuthority => "SetAuthority",
            UpgradeableLoaderInstruction::Close { .. } => "Close",
            UpgradeableLoaderInstruction::ExtendProgram { .. } => "ExtendProgram",
            UpgradeableLoaderInstruction::SetAuthorityChecked => "SetAuthorityChecked",
            UpgradeableLoaderInstruction::Upgrade { .. } => unreachable!(),
        }),
    }
}

/// The official state header, decoded from exactly its own bytes.
fn state(header: &[u8]) -> Result<UpgradeableLoaderState> {
    wincode::deserialize::<UpgradeableLoaderState>(header)
        .map_err(|error| anyhow::anyhow!("loader state does not decode: {error}"))
}

/// `UpgradeableLoaderState::Program`.
pub fn decode_program(data: &[u8]) -> Result<Address> {
    ensure!(
        data.len() == UpgradeableLoaderState::size_of_program(),
        "a loader Program account is {} bytes, not {}",
        UpgradeableLoaderState::size_of_program(),
        data.len()
    );
    match state(data)? {
        UpgradeableLoaderState::Program {
            programdata_address,
        } => Ok(programdata_address),
        other => bail!("account is loader state {other:?}, not Program"),
    }
}

/// A loader Buffer: its authority and every byte `Upgrade` would deploy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Buffer {
    pub authority: Option<Address>,
    /// Everything after the 37-byte metadata. `Upgrade` copies all of it,
    /// including any slack a larger-than-needed buffer was allocated with, so
    /// that is the artefact — not a trimmed ELF.
    pub bytes: Vec<u8>,
}

pub fn decode_buffer(data: &[u8]) -> Result<Buffer> {
    let metadata = UpgradeableLoaderState::size_of_buffer_metadata();
    ensure!(
        data.len() > metadata,
        "a loader Buffer holding a program is longer than its {metadata}-byte metadata"
    );
    let authority = match data[4] {
        // `None` serializes as one tag byte; the loader still reserves the key.
        0 => {
            ensure!(
                data[5..metadata].iter().all(|b| *b == 0),
                "a Buffer with no authority carries a non-zero authority slot"
            );
            state(&data[..5])?
        }
        1 => state(&data[..metadata])?,
        _ => bail!("Buffer authority tag {} is not an Option", data[4]),
    };
    match authority {
        UpgradeableLoaderState::Buffer { authority_address } => Ok(Buffer {
            authority: authority_address,
            bytes: data[metadata..].to_vec(),
        }),
        other => bail!("account is loader state {other:?}, not Buffer"),
    }
}

/// A ProgramData header and the bytes after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramData {
    pub deploy_slot: u64,
    pub upgrade_authority: Option<Address>,
    /// As `versions` reads it: everything after the header, padding kept.
    pub bytes: Vec<u8>,
}

pub fn decode_programdata(data: &[u8]) -> Result<ProgramData> {
    let metadata = UpgradeableLoaderState::size_of_programdata_metadata();
    ensure!(
        data.len() >= metadata,
        "ProgramData is shorter than its {metadata}-byte header"
    );
    let header = match data[12] {
        0 => {
            ensure!(
                data[13..metadata].iter().all(|b| *b == 0),
                "ProgramData with no authority carries a non-zero authority slot"
            );
            state(&data[..13])?
        }
        1 => state(&data[..metadata])?,
        _ => bail!("ProgramData authority tag {} is not an Option", data[12]),
    };
    match header {
        UpgradeableLoaderState::ProgramData {
            slot,
            upgrade_authority_address,
        } => Ok(ProgramData {
            deploy_slot: slot,
            upgrade_authority: upgrade_authority_address,
            bytes: data[metadata..].to_vec(),
        }),
        other => bail!("account is loader state {other:?}, not ProgramData"),
    }
}

/// Serializers for tests and fixtures. The inverse of the decoders above,
/// through the same official types.
pub mod encode {
    use super::*;

    pub fn upgrade() -> Vec<u8> {
        3u32.to_le_bytes().to_vec()
    }

    pub fn instruction(instruction: &UpgradeableLoaderInstruction) -> Vec<u8> {
        wincode::serialize(instruction).expect("loader instruction serializes")
    }

    pub fn program(programdata: &Address) -> Vec<u8> {
        wincode::serialize(&UpgradeableLoaderState::Program {
            programdata_address: *programdata,
        })
        .expect("program state serializes")
    }

    pub fn buffer(authority: Option<Address>, bytes: &[u8]) -> Vec<u8> {
        let mut data = wincode::serialize(&UpgradeableLoaderState::Buffer {
            authority_address: authority,
        })
        .expect("buffer state serializes");
        data.resize(UpgradeableLoaderState::size_of_buffer_metadata(), 0);
        data.extend_from_slice(bytes);
        data
    }

    pub fn programdata(deploy_slot: u64, authority: Option<Address>, bytes: &[u8]) -> Vec<u8> {
        let mut data = wincode::serialize(&UpgradeableLoaderState::ProgramData {
            slot: deploy_slot,
            upgrade_authority_address: authority,
        })
        .expect("programdata state serializes");
        data.resize(UpgradeableLoaderState::size_of_programdata_metadata(), 0);
        data.extend_from_slice(bytes);
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrade_is_decoded_only_in_its_feature_independent_forms() {
        assert_eq!(
            decode_instruction(&encode::upgrade()),
            LoaderInstruction::Upgrade
        );
        assert_eq!(
            decode_instruction(&encode::instruction(
                &UpgradeableLoaderInstruction::Upgrade { close_buffer: true }
            )),
            LoaderInstruction::Upgrade
        );
        assert_eq!(
            decode_instruction(&encode::instruction(
                &UpgradeableLoaderInstruction::Upgrade {
                    close_buffer: false
                }
            )),
            LoaderInstruction::UpgradeKeepingBuffer
        );
        assert_eq!(
            decode_instruction(&[3, 0, 0, 0, 1, 9]),
            LoaderInstruction::Undecodable
        );
        assert_eq!(
            decode_instruction(&[3, 0, 0, 0, 2]),
            LoaderInstruction::Undecodable
        );
        assert_eq!(
            decode_instruction(&[3, 0, 0]),
            LoaderInstruction::Undecodable
        );
        assert_eq!(
            decode_instruction(&encode::instruction(
                &UpgradeableLoaderInstruction::SetAuthority
            )),
            LoaderInstruction::Other("SetAuthority")
        );
    }

    #[test]
    fn states_round_trip_through_the_official_types() {
        let key = Address::from([7u8; 32]);
        assert_eq!(decode_program(&encode::program(&key)).unwrap(), key);
        for authority in [None, Some(key)] {
            let buffer = decode_buffer(&encode::buffer(authority, b"\x7fELF body")).unwrap();
            assert_eq!(buffer.authority, authority);
            assert_eq!(buffer.bytes, b"\x7fELF body");
            let programdata =
                decode_programdata(&encode::programdata(9, authority, b"elf")).unwrap();
            assert_eq!(programdata.upgrade_authority, authority);
            assert_eq!(programdata.deploy_slot, 9);
            assert_eq!(programdata.bytes, b"elf");
        }
        // One state is never read as another.
        assert!(decode_buffer(&encode::programdata(9, Some(key), b"elf")).is_err());
        assert!(decode_programdata(&encode::buffer(Some(key), b"elf plus")).is_err());
        let mut junk_slot = encode::buffer(None, b"elf");
        junk_slot[9] = 1;
        assert!(decode_buffer(&junk_slot).is_err());
    }

    #[test]
    fn programdata_is_the_interface_derivation() {
        let program: Address = "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy"
            .parse()
            .unwrap();
        assert_eq!(
            programdata_address(&program),
            Address::find_program_address(&[program.as_ref()], &id()).0
        );
    }
}
