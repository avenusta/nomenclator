use std::collections::BTreeMap;

use serde::Serialize;

use crate::lexical;
use crate::model::Finding;
use crate::rules::Rules;

/// Provider IDs found in a text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderIds {
    /// The ID of each provider, keyed by provider. Two different IDs of one provider are
    /// ambiguous.
    pub provider_ids: BTreeMap<String, Finding<String>>,
}

impl Rules {
    /// Reads provider IDs from the bracketed tags in any name or path text.
    pub fn parse_provider_ids(&self, text: &str) -> ProviderIds {
        ProviderIds {
            provider_ids: lexical::provider_ids(self, text, true),
        }
    }
}
