use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct User {
    pub user_id: String,  // PRIMARY IDENTIFIER - matches MongoDB structure
    pub email: String,
    pub name: String,
}
