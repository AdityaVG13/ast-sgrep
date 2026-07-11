use crate::intent::{ChannelWeights, QueryIntent};
pub const FUSION_TABLE_VERSION: &str = "1";
pub fn weights(_language: Option<&str>, intent: QueryIntent) -> ChannelWeights {
    match intent {
        QueryIntent::Conceptual => ChannelWeights {
            lexical: 1.05,
            graph: 0.86,
            ..ChannelWeights::default()
        },
        _ => ChannelWeights::default(),
    }
}
