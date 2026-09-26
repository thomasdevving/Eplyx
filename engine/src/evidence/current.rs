//! Measurements across a captured local execution, not validator S-1/S proof.
//! Account identity and token quantities use the same primitives as historical
//! evidence. Absent accounts remain absent; callers must interpret creation.
use super::{
    account::{self, AccountDelta},
    token,
};
use crate::{executor::ProbeTransactionExecution, types::NamedAccount};
use anyhow::{ensure, Context, Result};
use std::collections::BTreeSet;

pub fn pair(
    pre: &[NamedAccount],
    watch: &[String],
    result: &ProbeTransactionExecution,
) -> Result<Vec<AccountDelta>> {
    let watched: BTreeSet<_> = watch.iter().collect();
    ensure!(watched.len() == watch.len(), "duplicate watched account");
    let mut seen = BTreeSet::new();
    ensure!(
        pre.iter().all(|a| seen.insert(&a.address)),
        "duplicate opening account"
    );
    ensure!(
        result.post_accounts.keys().all(|a| watched.contains(a)),
        "unwatched closing account"
    );
    let opening: Vec<_> = pre
        .iter()
        .filter(|a| watched.contains(&a.address))
        .map(|a| NamedAccount {
            label: a.address.clone(),
            address: a.address.clone(),
            account: a.account.clone(),
        })
        .collect();
    Ok(account::pair_snapshots(
        "captured-local-execution",
        &opening,
        &result.post_accounts,
    ))
}

/// Optional quantities retain account creation/closure instead of inventing a
/// zero opening or closing balance. A delta exists only across two observations.
pub struct TokenAmounts {
    pub before: Option<u64>,
    pub after: Option<u64>,
    pub withheld_before: Option<u64>,
    pub withheld_after: Option<u64>,
    pub delta: Option<i128>,
}
pub fn token_amounts(
    pairs: &[AccountDelta],
    address: &str,
    program: &str,
    mint: &str,
) -> Result<TokenAmounts> {
    let Some(pair) = pairs.iter().find(|p| p.label() == address) else {
        return Ok(TokenAmounts {
            before: None,
            after: None,
            withheld_before: None,
            withheld_after: None,
            delta: None,
        });
    };
    let decode = |a: &crate::types::AccountSnapshot| -> Result<(u64, u64)> {
        ensure!(
            a.owner == program && !a.executable,
            "token program or executable changed"
        );
        let reader = token::TokenProgram::of(program).context("unsupported token program")?;
        ensure!(
            reader.account_mint(&a.data).as_deref() == Some(mint),
            "token mint changed"
        );
        crate::standard_programs::token::account_amounts(program, &a.data)
    };
    let before = pair.before.as_ref().map(decode).transpose()?;
    let after = pair.after.as_ref().map(decode).transpose()?;
    let delta = if before.is_some() && after.is_some() {
        Some(
            token::balance_deltas(std::slice::from_ref(pair))
                .first()
                .context("token balance cannot be measured")?
                .delta,
        )
    } else {
        None
    };
    Ok(TokenAmounts {
        before: before.map(|p| p.0),
        after: after.map(|p| p.0),
        withheld_before: before.map(|p| p.1),
        withheld_after: after.map(|p| p.1),
        delta,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::tests::snapshot;
    use crate::standard_programs::spl_token;
    use std::collections::BTreeMap;
    fn account(address: &str, balance: u64) -> NamedAccount {
        let mut bytes = vec![0; 165];
        bytes[..32].fill(1);
        bytes[32..64].fill(2);
        bytes[64..72].copy_from_slice(&balance.to_le_bytes());
        bytes[108] = 1;
        NamedAccount {
            label: "display label".into(),
            address: address.into(),
            account: snapshot(spl_token::PROGRAM_ID, 1, bytes),
        }
    }
    fn result(accounts: &[NamedAccount]) -> ProbeTransactionExecution {
        ProbeTransactionExecution {
            success: true,
            error: None,
            compute_units: 0,
            transaction_fee_lamports: 0,
            logs: vec![],
            inner_instructions: vec![],
            post_accounts: accounts
                .iter()
                .map(|a| (a.address.clone(), a.account.clone()))
                .collect::<BTreeMap<_, _>>(),
        }
    }
    #[test]
    fn reordered_current_accounts_pair_by_address_and_preserve_absence() {
        let pre = [account("z", 9), account("a", 3)];
        let closing = result(&[account("a", 8), account("z", 9), account("new", 7)]);
        let pairs = pair(&pre, &["z".into(), "a".into(), "new".into()], &closing).unwrap();
        let mint = bs58::encode([1; 32]).into_string();
        let a = token_amounts(&pairs, "a", spl_token::PROGRAM_ID, &mint).unwrap();
        assert_eq!(a.delta, Some(5));
        let new = token_amounts(&pairs, "new", spl_token::PROGRAM_ID, &mint).unwrap();
        assert_eq!((new.before, new.after, new.delta), (None, Some(7), None));
        let absent = token_amounts(&pairs, "absent", spl_token::PROGRAM_ID, &mint).unwrap();
        assert_eq!(
            (absent.before, absent.after, absent.delta),
            (None, None, None)
        );
        assert!(pair(&pre, &["z".into()], &closing).is_err());
        assert!(pair(&pre, &["z".into(), "z".into()], &result(&[])).is_err());
        assert!(pair(&[pre[0].clone(), pre[0].clone()], &[], &result(&[])).is_err());
    }
    #[test]
    fn changed_token_identity_and_malformed_state_never_become_balance_movements() {
        let pre = [account("a", 3)];
        let mint = bs58::encode([1; 32]).into_string();
        for variant in 0..3 {
            let mut post = account("a", 8);
            match variant {
                0 => post.account.data[..32].fill(9),
                1 => post.account.owner = "11111111111111111111111111111111".into(),
                _ => post.account.data[108] = 9,
            }
            let pairs = pair(&pre, &["a".into()], &result(&[post])).unwrap();
            assert!(token_amounts(&pairs, "a", spl_token::PROGRAM_ID, &mint).is_err());
        }
    }
}
