use hermes_tasks::UserId;

use crate::types::{Account, StoredAccount};

#[derive(Debug, Clone, Default)]
pub struct AccountStore {
    pub accounts: Vec<StoredAccount>,
    pub active_user_id: Option<UserId>,
    pub active_account: Option<Account>,
}

impl AccountStore {
    pub fn set_active(&mut self, user_id: UserId) {
        self.active_user_id = Some(user_id);
    }

    pub fn add_stored(&mut self, stored: StoredAccount) {
        if !self.accounts.iter().any(|a| a.user_id == stored.user_id) {
            self.accounts.push(stored);
        }
    }
}
