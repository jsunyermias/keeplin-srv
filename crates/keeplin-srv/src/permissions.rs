use uuid::Uuid;

use crate::{error::AppError, store::Note};

/// A capability bitset. Higher capabilities **imply** the lower ones (see
/// [`Capabilities::normalized`]), so a grant is always stored and compared in its expanded
/// form and there is no way to hold, say, `WRITE` without `READ`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities(i32);

impl Capabilities {
    pub const READ: i32 = 1;
    pub const WRITE: i32 = 2;
    pub const SHARE_READ: i32 = 4;
    pub const SHARE_WRITE: i32 = 8;
    pub const MANAGE: i32 = 16;
    /// Every capability bit (what an owner or a `manage` grant expands to).
    pub const ALL: i32 =
        Self::READ | Self::WRITE | Self::SHARE_READ | Self::SHARE_WRITE | Self::MANAGE;

    /// Build from a raw bitmask, expanding implied bits.
    pub fn from_bits(bits: i32) -> Self {
        Self(bits & Self::ALL).normalized()
    }

    /// No capabilities.
    pub fn empty() -> Self {
        Self(0)
    }

    /// The full set — an owner, or a `MANAGE` grant.
    pub fn all() -> Self {
        Self(Self::ALL)
    }

    /// Expand implied bits so containment checks are a plain mask test:
    /// `MANAGE` ⊃ everything, `SHARE_WRITE` ⊃ `SHARE_READ` + `WRITE`, and both
    /// `SHARE_READ` and `WRITE` ⊃ `READ`.
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

    /// The stored/serialised bitmask (already normalised).
    pub fn bits(self) -> i32 {
        self.0
    }

    fn contains(self, bit: i32) -> bool {
        self.0 & bit == bit
    }

    pub fn can_read(self) -> bool {
        self.contains(Self::READ)
    }
    pub fn can_write(self) -> bool {
        self.contains(Self::WRITE)
    }
    /// May see who a note is shared with.
    pub fn can_share_read(self) -> bool {
        self.contains(Self::SHARE_READ)
    }
    /// May grant/revoke shares (up to their own capabilities).
    pub fn can_share_write(self) -> bool {
        self.contains(Self::SHARE_WRITE)
    }
    pub fn can_manage(self) -> bool {
        self.contains(Self::MANAGE)
    }
}

/// A user's effective access to an entity: their capabilities plus whether they are the
/// **owner** (a separate, transferable status that no capability grant confers).
#[derive(Debug, Clone, Copy)]
pub struct Access {
    pub caps: Capabilities,
    pub is_owner: bool,
}

impl Access {
    /// The owner: every capability, plus ownership-only powers (delete, transfer).
    pub fn owner() -> Self {
        Self {
            caps: Capabilities::all(),
            is_owner: true,
        }
    }

    fn granted(caps: Capabilities) -> Self {
        Self {
            caps,
            is_owner: false,
        }
    }

    pub fn can_read(self) -> bool {
        self.caps.can_read()
    }
    pub fn can_write(self) -> bool {
        self.caps.can_write()
    }
    pub fn can_share_write(self) -> bool {
        self.caps.can_share_write()
    }
    /// Deleting a note and transferring its ownership are reserved to the owner.
    pub fn can_delete(self) -> bool {
        self.is_owner
    }
    pub fn can_transfer_ownership(self) -> bool {
        self.is_owner
    }
}

/// Resolve `user_id`'s [`Access`] to `note`, or `Forbidden` if they have none.
///
/// The owner is read from `note.owner_id`; everyone else's capabilities come from their
/// `note_shares` row (already the destructive-cascade result, so no notebook fallback is
/// needed at read time).
pub async fn resolve_note_access(
    store: &crate::store::Store,
    note: &Note,
    user_id: Uuid,
) -> Result<Access, AppError> {
    if note.owner_id == user_id {
        return Ok(Access::owner());
    }
    match store.get_share(note.id, user_id).await? {
        Some(share) => Ok(Access::granted(Capabilities::from_bits(share.capabilities))),
        None => Err(AppError::Forbidden),
    }
}

#[cfg(test)]
mod tests {
    use super::Capabilities as C;

    #[test]
    fn higher_bits_imply_lower_ones() {
        // write ⊃ read
        assert!(C::from_bits(C::WRITE).can_read());
        // share_read ⊃ read
        assert!(C::from_bits(C::SHARE_READ).can_read());
        // share_write ⊃ share_read + write + read
        let sw = C::from_bits(C::SHARE_WRITE);
        assert!(sw.can_share_read() && sw.can_write() && sw.can_read());
        // manage ⊃ everything
        let m = C::from_bits(C::MANAGE);
        assert!(m.can_read() && m.can_write() && m.can_share_read() && m.can_share_write());
        assert_eq!(m.bits(), C::ALL);
    }

    #[test]
    fn read_alone_implies_nothing_more() {
        let r = C::from_bits(C::READ);
        assert!(r.can_read());
        assert!(!r.can_write() && !r.can_share_read() && !r.can_manage());
    }

    #[test]
    fn unknown_bits_are_masked_off() {
        // A bit outside ALL is dropped, leaving a clean normalised set.
        let c = C::from_bits(C::WRITE | 0x4000);
        assert_eq!(c.bits(), C::WRITE | C::READ);
    }

    #[test]
    fn owner_has_every_capability() {
        assert_eq!(C::all().bits(), C::ALL);
    }
}
