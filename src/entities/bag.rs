use std::hash::Hash;

use super::{file::File, ticket::Ticket};

#[derive(Clone, Debug)]
pub struct Bag {
    pub id: String,
    pub ticket: Ticket,
    pub files: Vec<File>,
    pub created_at: String,
    pub updated_at: String,
}

impl PartialEq for Bag {
    fn eq(&self, other: &Self) -> bool {
        return self.id == other.id
            && self.ticket == other.ticket
            && self.files == other.files
            && self.created_at == other.created_at
            && self.updated_at == other.updated_at;
    }
}

impl Hash for Bag {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}
