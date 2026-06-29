use hermes_tasks::UserId;

use crate::types::ConsentRecord;

pub struct ConsentStore;

impl ConsentStore {
    pub fn is_granted(&self, _user_id: &UserId, _provider_id: &str) -> bool {
        false
    }

    pub fn record(&self, _record: &ConsentRecord) {}
}
