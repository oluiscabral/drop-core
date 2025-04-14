use std::hash::Hash;

#[derive(Clone, Debug)]
pub struct File {
    pub id: String,
    pub name: String,
    pub data: Vec<u8>,
    pub created_at: String,
    pub updated_at: String,
}

impl PartialEq for File {
    fn eq(&self, other: &Self) -> bool {
        return self.id == other.id
            && self.name == other.name
            && self.data == other.data
            && self.created_at == other.created_at
            && self.updated_at == other.updated_at;
    }
}

impl Hash for File {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}
