use util::encoding::codec;

use crate::ignition_proto::admin;

#[codec(schema = false)]
#[derive(Clone)]
pub struct User {
    pub id: String,
    pub name: String,
    pub status: UserStatus,
}

#[codec(schema = false)]
#[derive(Debug, PartialEq, Clone)]
pub enum UserStatus {
    Active,
    Inactive,
}

impl From<&UserStatus> for i32 {
    fn from(status: &UserStatus) -> Self {
        match status {
            UserStatus::Active => admin::user::Status::Active as i32,
            UserStatus::Inactive => admin::user::Status::Inactive as i32,
        }
    }
}

impl From<&User> for admin::User {
    fn from(user: &User) -> Self {
        admin::User {
            id: user.id.clone(),
            name: user.name.clone(),
            status: (&user.status).into(),
        }
    }
}
