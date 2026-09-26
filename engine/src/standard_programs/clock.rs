//! Captured Solana Clock sysvar fields and exact bytes.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use solana_clock::Clock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedClock {
    #[serde(with = "crate::numfmt::u64_string")]
    pub slot: u64,
    #[serde(with = "crate::numfmt::i64_string")]
    pub epoch_start_timestamp: i64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub epoch: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub leader_schedule_epoch: u64,
    #[serde(with = "crate::numfmt::i64_string")]
    pub unix_timestamp: i64,
}

impl From<&Clock> for CapturedClock {
    fn from(clock: &Clock) -> Self {
        Self {
            slot: clock.slot,
            epoch_start_timestamp: clock.epoch_start_timestamp,
            epoch: clock.epoch,
            leader_schedule_epoch: clock.leader_schedule_epoch,
            unix_timestamp: clock.unix_timestamp,
        }
    }
}

impl CapturedClock {
    pub fn clock(&self) -> Clock {
        Clock {
            slot: self.slot,
            epoch_start_timestamp: self.epoch_start_timestamp,
            epoch: self.epoch,
            leader_schedule_epoch: self.leader_schedule_epoch,
            unix_timestamp: self.unix_timestamp,
        }
    }
    pub fn bytes(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(40);
        data.extend_from_slice(&self.slot.to_le_bytes());
        data.extend_from_slice(&self.epoch_start_timestamp.to_le_bytes());
        data.extend_from_slice(&self.epoch.to_le_bytes());
        data.extend_from_slice(&self.leader_schedule_epoch.to_le_bytes());
        data.extend_from_slice(&self.unix_timestamp.to_le_bytes());
        data
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() == 40, "malformed Clock sysvar");
        let u = |i: usize| {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[i..i + 8]);
            u64::from_le_bytes(word)
        };
        Ok(Self {
            slot: u(0),
            epoch_start_timestamp: u(8) as i64,
            epoch: u(16),
            leader_schedule_epoch: u(24),
            unix_timestamp: u(32) as i64,
        })
    }
}
