use std::hash::Hash;

#[derive(Clone, Debug)]
pub struct Ticket {
    pub id: String,
}

impl PartialEq for Ticket {
    fn eq(&self, other: &Self) -> bool {
        return self.id == other.id;
    }
}

impl Hash for Ticket {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}
