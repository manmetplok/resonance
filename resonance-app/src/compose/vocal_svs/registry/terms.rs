//! What the registry knows of the plok.org index (the typo guard's ids,
//! the hashes it pins) and the terms each bank is used under.

use super::{builtin, BuiltinBank, PlokIndex, VoicebankRegistry};

/// What the registry keeps of one index entry.
#[derive(Debug, Clone, Default)]
pub(super) struct IndexBank {
    /// Its `languages` name `en`, or name nothing.
    pub(super) english: bool,
    /// The pinned upstream file, lowercase.
    pub(super) sha256: Option<String>,
}

/// The terms a bank is used under, as the readouts show them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BankTerms {
    pub credit: Option<String>,
    pub terms_url: Option<String>,
    /// `free_with_credit`, `paid` or `permission`.
    pub commercial: Option<String>,
}

impl VoicebankRegistry {
    /// Remember the index's banks (see [`Self::in_index`]).
    pub fn set_index(&mut self, index: &PlokIndex) {
        self.index_banks = index
            .voicebanks
            .iter()
            .map(|b| {
                let english = b.languages.is_empty()
                    || b.languages.iter().any(|l| l.eq_ignore_ascii_case("en"));
                let sha256 = b.sha256.as_ref().map(|s| s.trim().to_ascii_lowercase());
                (b.id.clone(), IndexBank { english, sha256 })
            })
            .collect();
        self.revision += 1;
    }

    /// `Some(sings English)` when the plok.org index lists bank `id`.
    pub fn in_index(&self, id: &str) -> Option<bool> {
        self.index_banks.get(id).map(|b| b.english)
    }

    /// The terms bank `id` is used under. An install record
    /// (`.resonance-plok.json`, only read when its `id` is its folder's
    /// name) carries the terms the user accepted, and wins — except on a
    /// built-in bank, where the built-in table wins unless the record pins
    /// the very file the index lists now (its sha256 is the index
    /// entry's): a record is a file anyone can write, and must not turn a
    /// "permission" bank into a free one.
    pub fn terms(&self, id: &str) -> BankTerms {
        let record = self.banks.get(id).and_then(|b| b.plok.as_ref());
        let builtin = builtin::builtin(id);
        let from_builtin = |f: fn(&BuiltinBank) -> Option<&'static str>| {
            builtin.and_then(f).map(str::to_owned)
        };
        let trusted = record.filter(|r| {
            builtin.is_none()
                || r.sha256.as_ref().is_some_and(|sha| {
                    self.index_banks
                        .get(id)
                        .and_then(|e| e.sha256.as_deref())
                        .is_some_and(|pinned| pinned.eq_ignore_ascii_case(sha))
                })
        });
        match trusted {
            Some(r) => BankTerms {
                credit: r.credit.clone().or_else(|| from_builtin(|b| b.credit)),
                terms_url: r.terms_url.clone().or_else(|| from_builtin(|b| b.terms_url)),
                commercial: r.commercial.clone().or_else(|| from_builtin(|b| b.commercial)),
            },
            None => BankTerms {
                credit: from_builtin(|b| b.credit).or_else(|| record?.credit.clone()),
                terms_url: from_builtin(|b| b.terms_url).or_else(|| record?.terms_url.clone()),
                commercial: from_builtin(|b| b.commercial)
                    .or_else(|| record.filter(|_| builtin.is_none())?.commercial.clone()),
            },
        }
    }
}
