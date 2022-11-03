use super::Hedger;

pub struct CypherHedger {}

impl Hedger for CypherHedger {
    fn pulse(&self) -> super::HedgerPulseResult {
        todo!()
    }
}
