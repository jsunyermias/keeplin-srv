// md:Overview
use uuid::Uuid;

use crate::{error::AppError, store::Note};

// md:Capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities(i32);

// md:impl Capabilities
impl Capabilities {
    pub const READ: i32 = 1;
    pub const WRITE: i32 = 2;
    pub const SHARE_READ: i32 = 4;
    pub const SHARE_WRITE: i32 = 8;
    pub const MANAGE: i32 = 16;
    pub const ALL: i32 =
        Self::READ | Self::WRITE | Self::SHARE_READ | Self::SHARE_WRITE | Self::MANAGE;

    // md:impl Capabilities > fn from_bits
    pub fn from_bits(bits: i32) -> Self {
        Self(bits & Self::ALL).normalized()
    }

    // md:impl Capabilities > fn empty
    pub fn empty() -> Self {
        Self(0)
    }

    // md:impl Capabilities > fn all
    pub fn all() -> Self {
        Self(Self::ALL)
    }

    // md:impl Capabilities > fn normalized
    fn normalized(self) -> Self {
        let mut b = self.0;
        if b & Self::MANAGE != 0 {
            b |= Self::ALL;
        }
        if b & Self::SHARE_WRITE != 0 {
            b |= Self::SHARE_READ | Self::WRITE;
        }
        if b & (Self::SHARE_READ | Self::WRITE) != 0 {
            b |= Self::READ;
        }
        Self(b)
    }

    // md:impl Capabilities > fn bits
    pub fn bits(self) -> i32 {
        self.0
    }

    // md:impl Capabilities > fn contains
    fn contains(self, bit: i32) -> bool {
        self.0 & bit == bit
    }

    // md:impl Capabilities > can_* accessors
    pub fn can_read(self) -> bool {
        self.contains(Self::READ)
    }
    pub fn can_write(self) -> bool {
        self.contains(Self::WRITE)
    }
    pub fn can_share_read(self) -> bool {
        self.contains(Self::SHARE_READ)
    }
    pub fn can_share_write(self) -> bool {
        self.contains(Self::SHARE_WRITE)
    }
    pub fn can_manage(self) -> bool {
        self.contains(Self::MANAGE)
    }
}

// md:Access
#[derive(Debug, Clone, Copy)]
pub struct Access {
    pub caps: Capabilities,
    pub is_owner: bool,
}

// md:impl Access
impl Access {
    // md:impl Access > fn owner
    pub fn owner() -> Self {
        Self {
            caps: Capabilities::all(),
            is_owner: true,
        }
    }

    // md:impl Access > fn granted
    fn granted(caps: Capabilities) -> Self {
        Self {
            caps,
            is_owner: false,
        }
    }

    // md:impl Access > accessors
    pub fn can_read(self) -> bool {
        self.caps.can_read()
    }
    pub fn can_write(self) -> bool {
        self.caps.can_write()
    }
    pub fn can_share_write(self) -> bool {
        self.caps.can_share_write()
    }
    pub fn can_delete(self) -> bool {
        self.is_owner
    }
    pub fn can_transfer_ownership(self) -> bool {
        self.is_owner
    }
}

// md:fn resolve_note_access
pub async fn resolve_note_access(
    store: &crate::store::Store,
    note: &Note,
    user_id: Uuid,
) -> Result<Access, AppError> {
    if note.owner_id == user_id {
        return Ok(Access::owner());
    }
    if let Some(nb) = note.notebook_id {
        if store.notebook_owner(nb).await? == Some(user_id) {
            return Ok(Access::granted(Capabilities::all()));
        }
    }
    match store.get_share(note.id, user_id).await? {
        Some(share) => Ok(Access::granted(Capabilities::from_bits(share.capabilities))),
        None => Err(AppError::Forbidden),
    }
}

// md:fn resolve_notebook_access
pub async fn resolve_notebook_access(
    store: &crate::store::Store,
    notebook_id: Uuid,
    user_id: Uuid,
) -> Result<Access, AppError> {
    let owner = store
        .notebook_owner(notebook_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if owner == user_id {
        return Ok(Access::owner());
    }
    match store.get_notebook_share(notebook_id, user_id).await? {
        Some(share) => Ok(Access::granted(Capabilities::from_bits(share.capabilities))),
        None => Err(AppError::Forbidden),
    }
}

// md:mod tests
#[cfg(test)]
mod tests {
    use super::Capabilities as C;

    // md:mod tests > fn higher_bits_imply_lower_ones
    #[test]
    fn higher_bits_imply_lower_ones() {
        assert!(C::from_bits(C::WRITE).can_read());
        assert!(C::from_bits(C::SHARE_READ).can_read());
        let sw = C::from_bits(C::SHARE_WRITE);
        assert!(sw.can_share_read() && sw.can_write() && sw.can_read());
        let m = C::from_bits(C::MANAGE);
        assert!(m.can_read() && m.can_write() && m.can_share_read() && m.can_share_write());
        assert_eq!(m.bits(), C::ALL);
    }

    // md:mod tests > fn read_alone_implies_nothing_more
    #[test]
    fn read_alone_implies_nothing_more() {
        let r = C::from_bits(C::READ);
        assert!(r.can_read());
        assert!(!r.can_write() && !r.can_share_read() && !r.can_manage());
    }

    // md:mod tests > fn unknown_bits_are_masked_off
    #[test]
    fn unknown_bits_are_masked_off() {
        let c = C::from_bits(C::WRITE | 0x4000);
        assert_eq!(c.bits(), C::WRITE | C::READ);
    }

    // md:mod tests > fn owner_has_every_capability
    #[test]
    fn owner_has_every_capability() {
        assert_eq!(C::all().bits(), C::ALL);
    }
}
