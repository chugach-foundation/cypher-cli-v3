use super::Hedger;

pub struct SerumHedger {}

impl Hedger for SerumHedger {
    fn pulse(&self) -> super::HedgerPulseResult {
        todo!()
    }
}
