#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Added,
    Modified,
    Deleted,
    Renamed { from: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// Repo-relative path; the destination for renames.
    pub path: String,
    pub status: Status,
}

impl Change {
    /// Every path git must see to record this change: the old and new path of a
    /// rename, otherwise just the path.
    pub fn pathspecs(&self) -> Vec<&str> {
        match &self.status {
            Status::Renamed { from } => vec![from.as_str(), self.path.as_str()],
            _ => vec![self.path.as_str()],
        }
    }
}
