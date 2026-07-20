# `permissions.rs` — note capabilities

Self-contained companion for `crates/keeplin-srv/src/permissions.rs`. It documents
**every code block of the source file, in source order** — a reader with only this file
must be able to understand `permissions.rs` without opening anything else, so
project-wide conventions are deliberately re-explained here (hyper-redundancy is
intended).

**How to navigate**: every block in `permissions.rs` carries exactly one marker comment
of the form `// md:<Header> > … > <Block header>`, whose path is the header chain of the
section documenting it here (starting below the file title). Grep the marker text to jump
code → doc; grep the section's block name (or the marker path) in the `.rs` to jump
doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the module's imports. Marker `// md:Overview` at
the top of the file.

**Code** — complete and verbatim:

```rust
// md:Overview
use uuid::Uuid;

use crate::{error::AppError, store::Note};
```

**What it does** — The **capability model** (Front B) and the two functions that
resolve a user's effective access to a note or a notebook. Pure authorization logic
with at most two database lookups; every note handler and the collaborative channel
resolve access here before reading or mutating —
`resolve_note_access`/`resolve_notebook_access` are the **single choke points** for
authorisation; handlers must not roll their own checks.

The model in brief: a grant is a **bitset of capabilities**, not a fixed role, with
higher bits implying lower ones (`READ`=1, `WRITE`=2 ⊃ READ, `SHARE_READ`=4 ⊃ READ,
`SHARE_WRITE`=8 ⊃ SHARE_READ+WRITE, `MANAGE`=16 ⊃ all). **Ownership is separate and
transferable**: the owner always has every capability *plus* the two owner-only powers
— delete and transfer — that no capability bit confers; ownership is never a share
row, so it cannot be revoked by removing a share.

Notebook interplay (the destructive cascade, implemented in `store.rs`): a notebook's
grants are **copied over** a child note's `note_shares` whenever the notebook's shares
change or a note is moved into the notebook (a move to the inbox — a null notebook —
leaves the note's shares untouched). Because grants are materialised per note,
`resolve_note_access` needs no notebook-share fallback at read time. The cascade
governs grants only; it never changes `owner_id`. Additionally, the **notebook owner
holds implicit `manage` over every child note** (folder-owner model) — resolved at
access time here, not materialised, so a notebook ownership transfer needs no share
rewrite. Moving a note into a notebook requires `write` on both the note and the
destination notebook (the move adopts the destination's grants and replaces the note's
own — consent on both sides); moving to the inbox needs no destination check.

Enforcement mapping used by callers: read (get/export/`Join`) → `can_read`; write
(patch, collaborative `Op`) → `can_write`; grant/revoke shares → `can_share_write`,
with granted bits **capped to the granter's own** (no privilege escalation; enforced
in `http.rs`); list shares → `can_share_read`; revoking your *own* share is always
allowed; delete/transfer → owner only. Sharing targets a user by email (resolved
server-side) or raw id.

**Dependencies** — `uuid` (external). Internal: `crate::error::AppError` (`error.rs`),
`crate::store::{Store, Note}` and the share lookups `get_share` /
`get_notebook_share` / `notebook_owner` (`store.rs`). Schema:
`migrations/0005_permissions.sql`, `0006_notebook_permissions.sql`.

**Used by** — `http.rs` (~15 call sites across note, share, notebook and history
handlers), `collab.rs` (`handle_msg` re-resolves access per `Join` and per `Op` —
issue #30: revocation must bite mid-connection), `store.rs` (stores the normalised
bitmasks; its `list_notes_for_user` mirrors the folder-owner rule in SQL).

**Repeated context** — Server-side authorisation composes with the data model like
this: all durable data is per-user (`user_id` scoping); sharing is the only
cross-user path, and it always flows through these capability rows. The collaborative
channel enforces the same `can_read`/`can_write` gates as REST, so no surface is
weaker than another.

---

## Capabilities

**Identification** — struct (newtype over `i32`); marker `// md:Capabilities`.

**Code** — complete and verbatim:

```rust
// md:Capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities(i32);
```

**What it does** — A normalised capability bitset. The private `i32` is only ever a
**normalised** mask (implied bits expanded), so containment checks are a single mask
test — cheap and free of role total-orderings. Constructed via `from_bits` /
`empty` / `all`, never directly.

**Dependencies** — none.

**Used by** — `Access.caps` (this file); `http.rs` (`Capabilities::from_bits` on
share-grant input, capping logic); `store.rs` stores `bits()` in
`note_shares.capabilities` / `notebook_shares.capabilities`.

**Repeated context** — Normalise-on-entry is the model's core trick: because every
stored/compared value has implied bits expanded, "does this grant allow X" never needs
to re-derive implications.

---

## impl Capabilities

**Identification** — inherent impl block; marker `// md:impl Capabilities`. Contains
the bit constants and the constructors/accessors documented below.

**Code** — container: members documented as sub-blocks below: consts, fn from_bits, fn empty, fn all, fn normalized, fn bits, fn contains, can_* accessors.

**What it does** — The capability bit constants (their own `consts` sub-block below),
the constructors (`from_bits`, `empty`, `all`), the normaliser, and the `can_*`
accessors.

**Dependencies** — `Capabilities` (this file).

**Used by** — see the method subsections.

**Repeated context** — none beyond the methods' own (below).

### consts

**Identification** — the associated bit constants of `impl Capabilities`
(`READ`/`WRITE`/`SHARE_READ`/`SHARE_WRITE`/`MANAGE`/`ALL`); marker
`// md:impl Capabilities > consts`.

**Code** — complete and verbatim:

```rust
    // md:impl Capabilities > consts
    pub const READ: i32 = 1;
    pub const WRITE: i32 = 2;
    pub const SHARE_READ: i32 = 4;
    pub const SHARE_WRITE: i32 = 8;
    pub const MANAGE: i32 = 16;
    pub const ALL: i32 =
        Self::READ | Self::WRITE | Self::SHARE_READ | Self::SHARE_WRITE | Self::MANAGE;
```

**What it does** — The five capability bits and their union `ALL`. Each named const is
a distinct power-of-two bit; the "implies" relationships in the table are materialised
by `normalized`, **not** by the raw values here (a raw const is just its own bit —
`WRITE == 2`, never `3`). `ALL` is defined as the OR of the five bits (`31`), so
appending a future bit to the list extends `ALL` automatically. Bit constants
(associated consts, part of this block):

| Const | Value | Implies | Meaning |
|-------|:-:|---------|---------|
| `READ` | 1 | — | see the note/notebook |
| `WRITE` | 2 | READ | edit it (lines, metadata) |
| `SHARE_READ` | 4 | READ | see who it is shared with |
| `SHARE_WRITE` | 8 | SHARE_READ, WRITE (→READ) | grant/revoke shares |
| `MANAGE` | 16 | everything | full control short of ownership |
| `ALL` | 31 | — | every bit; what an owner or a `MANAGE` grant expands to |

**Dependencies** — none (plain `i32` associated consts). `ALL` references the other
five consts; expects each to stay a distinct single bit so their OR is the full mask.

**Used by** — `from_bits` (`& Self::ALL` masks unknown bits), `all` (returns
`Self(ALL)`), `normalized` (tests the `SHARE_WRITE`/`SHARE_READ`/`WRITE` bits);
`http.rs` (share-grant capping) and `store.rs` (persisted bitmasks). Fase 3.5 will
add `FULL_CONTROL` here.

**Repeated context** — Capability bit values are a wire/storage compatibility
surface: `store.rs` persists them in share rows, so a value must never be renumbered —
only new bits appended (and folded into `ALL`).

### fn from_bits

**Identification** — associated function; marker
`// md:impl Capabilities > fn from_bits`. `pub fn from_bits(bits: i32) -> Self`.

**Code** — complete and verbatim:

```rust
    // md:impl Capabilities > fn from_bits
    pub fn from_bits(bits: i32) -> Self {
        Self(bits & Self::ALL).normalized()
    }
```

**What it does** — Builds from a raw bitmask: masks off unknown bits (`& ALL`), then
expands implied bits (`normalized`). The only entry point for untrusted masks (share
bodies from clients, rows from the database).

**Dependencies** — `normalized` (this file).

**Used by** — `http.rs` (share create/update request bodies, both note and notebook),
`resolve_note_access` / `resolve_notebook_access` (stored rows), unit tests.

**Repeated context** — Unknown bits are dropped, not rejected: forward compatibility
for clients sending newer masks; the surviving grant is still well-formed.

### fn empty

**Identification** — associated function; marker
`// md:impl Capabilities > fn empty`. `pub fn empty() -> Self` — no capabilities.

**Code** — complete and verbatim:

```rust
    // md:impl Capabilities > fn empty
    pub fn empty() -> Self {
        Self(0)
    }
```

**What it does / Dependencies / Used by** — the zero mask; used by `http.rs` when
computing capability intersections/caps.

**Repeated context** — none.

### fn all

**Identification** — associated function; marker `// md:impl Capabilities > fn all`.
`pub fn all() -> Self` — the full set.

**Code** — complete and verbatim:

```rust
    // md:impl Capabilities > fn all
    pub fn all() -> Self {
        Self(Self::ALL)
    }
```

**What it does** — Every bit: what an owner holds, and what a `MANAGE` grant expands
to.

**Dependencies** — `ALL` (this block).

**Used by** — `Access::owner` (this file), `resolve_note_access` (the notebook-owner
implicit-manage branch), unit tests.

**Repeated context** — Full capabilities ≠ ownership: `all()` never confers
delete/transfer (those live on `Access::is_owner`).

### fn normalized

**Identification** — private method; marker
`// md:impl Capabilities > fn normalized`. `fn normalized(self) -> Self`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Expands implied bits so containment checks are a plain mask test:
`MANAGE` ⊃ everything; `SHARE_WRITE` ⊃ `SHARE_READ` + `WRITE`; `SHARE_READ` or
`WRITE` ⊃ `READ`. Idempotent.

**Dependencies** — the bit constants (this block).

**Used by** — `from_bits` (this file) only.

**Repeated context** — The implication chain is the *definition* of the hierarchy;
`store.rs` persists only already-normalised masks, so this function is the single
place the hierarchy is encoded.

### fn bits

**Identification** — method; marker `// md:impl Capabilities > fn bits`.
`pub fn bits(self) -> i32` — the stored/serialised bitmask (already normalised).

**Code** — complete and verbatim:

```rust
    // md:impl Capabilities > fn bits
    pub fn bits(self) -> i32 {
        self.0
    }
```

**What it does / Dependencies / Used by** — raw accessor for persistence and JSON;
used by `http.rs` (responses, capping arithmetic) and `store.rs` (column values).

**Repeated context** — Values leaving through here are always normalised — consumers
may mask-test without re-normalising.

### fn contains

**Identification** — private method; marker
`// md:impl Capabilities > fn contains`. `fn contains(self, bit: i32) -> bool`.

**Code** — complete and verbatim:

```rust
    // md:impl Capabilities > fn contains
    fn contains(self, bit: i32) -> bool {
        self.0 & bit == bit
    }
```

**What it does** — `self.0 & bit == bit` — the single mask test backing every
`can_*`.

**Dependencies / Used by** — the `can_*` accessors (below).

**Repeated context** — none.

### can_* accessors

**Identification** — five public methods; one marker for the group:
`// md:impl Capabilities > can_* accessors`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — The capability queries callers use; each is one `contains` test
on the normalised mask.

**Dependencies** — `contains` (this file).

**Used by** — `Access`'s forwarding methods (this file); `http.rs` (share-list and
cap checks); unit tests.

**Repeated context** — The enforcement mapping (which handler checks which `can_*`)
is tabulated in *Overview*; keep the two in sync when adding endpoints.

---

## Access

**Identification** — struct; marker `// md:Access`.

**Code** — complete and verbatim:

```rust
// md:Access
#[derive(Debug, Clone, Copy)]
pub struct Access {
    pub caps: Capabilities,
    pub is_owner: bool,
}
```

**What it does** — A user's **effective access** to an entity: their capabilities
plus whether they are the owner — a separate, transferable status that no capability
grant confers. Owner-only powers (delete, transfer ownership) key off `is_owner`,
never off capability bits.

**Dependencies** — `Capabilities` (this file).

**Used by** — returned by both resolvers (this file); consumed by `http.rs` (every
note/notebook handler) and `collab.rs` (`Join`/`Op` gating). `http.rs` also passes
`&Access` into its history-visibility helper.

**Repeated context** — Ownership vs capabilities, restated: the cascade and shares
move capability bits around; `owner_id` moves only via the explicit transfer
endpoint. Conflating the two would let a `manage` grantee delete someone's note.

---

## impl Access

**Identification** — inherent impl block; marker `// md:impl Access`. Constructors
and forwarding accessors documented below.

**Code** — container: members documented as sub-blocks below: fn owner, fn granted, accessors.

**What it does** — Construction (`owner`, `granted`) plus capability forwarding and
the owner-only checks.

**Dependencies** — `Access`, `Capabilities` (this file).

**Used by** — see subsections.

**Repeated context** — none beyond the methods' own (below).

### fn owner

**Identification** — associated function; marker `// md:impl Access > fn owner`.
`pub fn owner() -> Self` — every capability, `is_owner: true`.

**Code** — complete and verbatim:

```rust
    // md:impl Access > fn owner
    pub fn owner() -> Self {
        Self {
            caps: Capabilities::all(),
            is_owner: true,
        }
    }
```

**What it does / Dependencies / Used by** — the owner's access value; built by both
resolvers when `user_id` matches the owner column.

**Repeated context** — Owner additionally holds delete/transfer via `is_owner` —
powers `Capabilities::all()` alone does not grant.

### fn granted

**Identification** — private associated function; marker
`// md:impl Access > fn granted`. `fn granted(caps: Capabilities) -> Self` —
`is_owner: false`.

**Code** — complete and verbatim:

```rust
    // md:impl Access > fn granted
    fn granted(caps: Capabilities) -> Self {
        Self {
            caps,
            is_owner: false,
        }
    }
```

**What it does / Dependencies / Used by** — wraps share-derived (or
implicit-manage) capabilities; used only by the two resolvers.

**Repeated context** — Private so no caller can fabricate an owner-less full-power
`Access` except through resolution.

### accessors

**Identification** — five public methods; one marker for the group:
`// md:impl Access > accessors`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — The first three forward to the capability mask; the last two are
`is_owner` — deleting a note and transferring its ownership are reserved to the
owner.

**Dependencies** — `Capabilities` accessors (this file).

**Used by** — `http.rs` handlers (get/patch/delete/transfer/share) and `collab.rs`
(`Join` needs `can_read`, `Op` needs `can_write`).

**Repeated context** — Soft-delete note: "delete" here means setting `deleted_at`
(tombstone) — the project never hard-deletes replicated entities — but even the
tombstone write is owner-only.

---

## fn resolve_note_access

**Identification** — public async function; marker `// md:fn resolve_note_access`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Resolves `user_id`'s `Access` to `note`, or `Forbidden` if they
have none. Order:

1. `note.owner_id == user_id` → `Access::owner()`.
2. The note is filed in a notebook and `user_id` owns that notebook
   (`store.notebook_owner`) → implicit `manage`: `granted(Capabilities::all())` —
   full capabilities but **not** ownership (delete/transfer stay with
   `note.owner_id`). Resolved here rather than materialised by the cascade so it
   survives notebook ownership transfers with no share rows to maintain.
3. Their `note_shares` row (`store.get_share`) → its (already normalised, already
   cascade-resolved) capabilities.
4. No row → `Err(AppError::Forbidden)`.

**Dependencies** — `Note`, `Store::{notebook_owner, get_share}` (`store.rs`);
`Access`/`Capabilities` (this file); `AppError` (`error.rs`).

**Used by** — `http.rs` (get/update/delete/export/import-into, share
create/list/revoke, transfer, history — ~13 sites), `collab.rs::handle_msg`
(per-`Join` and per-`Op`, so a revocation takes effect mid-connection — issue #30).

**Repeated context** — No notebook-share fallback is needed at read time because the
destructive cascade (in `store.rs`) already materialised notebook grants onto
`note_shares` — the folder-owner implicit `manage` is the deliberate exception,
computed live.

---

## fn resolve_notebook_access

**Identification** — public async function; marker
`// md:fn resolve_notebook_access`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — Resolves `user_id`'s `Access` to a notebook. Missing notebook →
`NotFound`. Owner (`notebooks.user_id`) → `Access::owner()`. Else their
`notebook_shares` row (`store.get_notebook_share`) → its capabilities; no row →
`Forbidden`.

**Dependencies** — `Store::{notebook_owner, get_notebook_share}` (`store.rs`);
`Access`/`Capabilities` (this file); `AppError` (`error.rs`).

**Used by** — `http.rs` notebook handlers (update/delete/share create/list/revoke,
notebook transfer, listing a notebook's notes), and the note-move path (moving a
note **into** a notebook requires `write` on the destination — see *Overview*).

**Repeated context** — Notebook shares are the *source* the destructive cascade
copies onto child notes; changing them triggers the cascade in the same transaction
(`store.rs`), which is why resolution here can stay a simple two-lookup function.

---

## mod tests

**Identification** — `#[cfg(test)]` unit-test module; marker `// md:mod tests`.
Four tests, below.

**Code** — container: members documented as sub-blocks below: fn higher_bits_imply_lower_ones, fn read_alone_implies_nothing_more, fn unknown_bits_are_masked_off, fn owner_has_every_capability.

**What it does** — Pure unit tests of the bitset algebra (no database).

**Dependencies** — `Capabilities` (aliased `C`).

**Used by** — `cargo test` only.

**Repeated context** — These pin the implication table in *impl Capabilities*.

### fn higher_bits_imply_lower_ones

**Identification** — `#[test]`; marker
`// md:mod tests > fn higher_bits_imply_lower_ones`.

**Code** — complete and verbatim:

```rust
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
```

**What it does** — write ⊃ read; share_read ⊃ read; share_write ⊃ share_read +
write + read; manage ⊃ everything (`bits() == ALL`).

**Dependencies / Used by** — `Capabilities`; `cargo test`.

**Repeated context** — Pins the implication chain.

### fn read_alone_implies_nothing_more

**Identification** — `#[test]`; marker
`// md:mod tests > fn read_alone_implies_nothing_more`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn read_alone_implies_nothing_more
    #[test]
    fn read_alone_implies_nothing_more() {
        let r = C::from_bits(C::READ);
        assert!(r.can_read());
        assert!(!r.can_write() && !r.can_share_read() && !r.can_manage());
    }
```

**What it does** — `READ` grants only read: no write, no share_read, no manage —
the hierarchy points downward only.

**Dependencies / Used by** — `Capabilities`; `cargo test`.

**Repeated context** — none.

### fn unknown_bits_are_masked_off

**Identification** — `#[test]`; marker
`// md:mod tests > fn unknown_bits_are_masked_off`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn unknown_bits_are_masked_off
    #[test]
    fn unknown_bits_are_masked_off() {
        let c = C::from_bits(C::WRITE | 0x4000);
        assert_eq!(c.bits(), C::WRITE | C::READ);
    }
```

**What it does** — A bit outside `ALL` (0x4000) is dropped by `from_bits`, leaving
the clean normalised set (`WRITE|READ`).

**Dependencies / Used by** — `Capabilities`; `cargo test`.

**Repeated context** — Pins forward-compatibility of grant masks.

### fn owner_has_every_capability

**Identification** — `#[test]`; marker
`// md:mod tests > fn owner_has_every_capability`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn owner_has_every_capability
    #[test]
    fn owner_has_every_capability() {
        assert_eq!(C::all().bits(), C::ALL);
    }
```

**What it does** — `Capabilities::all().bits() == ALL`.

**Dependencies / Used by** — `Capabilities`; `cargo test`.

**Repeated context** — none.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the
navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh with
`graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `resolve_note_access()` — defined here (EXTRACTED; 13 cross-file edge(s))
- `resolve_notebook_access()` — defined here (EXTRACTED; 8 cross-file edge(s))
- `Access` — defined here (EXTRACTED; 1 cross-file edge(s))
- `Capabilities` — defined here (EXTRACTED; file-local)
- `.from_bits()` — defined here (EXTRACTED; file-local)
- `.empty()` — defined here (EXTRACTED; file-local)
- `.all()` — defined here (EXTRACTED; file-local)
- `.normalized()` — defined here (EXTRACTED; file-local)
- `.bits()` — defined here (EXTRACTED; file-local)
- `.contains()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/error.rs` — the API error type (EXTRACTED: references×2; e.g. `AppError`)
- `crates/keeplin-srv/src/store.rs` — the PostgreSQL data-access layer (EXTRACTED: references×3; e.g. `Note`, `Store`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/collab.rs` — the collaborative session engine (EXTRACTED: calls×1; e.g. `handle_msg()`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×15, references×1; e.g. `get_note()`, `update_note()`, `delete_note()`)

## Coverage checklist

Every code block of `permissions.rs`, in source order, each documented above (five
points) and carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | imports (`use …`) | `// md:Overview` | Overview |
| 2 | `struct Capabilities` | `// md:Capabilities` | Capabilities |
| 3 | `impl Capabilities` | `// md:impl Capabilities` | impl Capabilities |
| 4 | `consts` (bit constants) | `// md:impl Capabilities > consts` | impl Capabilities › consts |
| 5 | `fn from_bits` | `// md:impl Capabilities > fn from_bits` | impl Capabilities › fn from_bits |
| 6 | `fn empty` | `// md:impl Capabilities > fn empty` | impl Capabilities › fn empty |
| 7 | `fn all` | `// md:impl Capabilities > fn all` | impl Capabilities › fn all |
| 8 | `fn normalized` | `// md:impl Capabilities > fn normalized` | impl Capabilities › fn normalized |
| 9 | `fn bits` | `// md:impl Capabilities > fn bits` | impl Capabilities › fn bits |
| 10 | `fn contains` | `// md:impl Capabilities > fn contains` | impl Capabilities › fn contains |
| 11 | `can_read`…`can_manage` (5 fns) | `// md:impl Capabilities > can_* accessors` | impl Capabilities › can_read / … / can_manage |
| 12 | `struct Access` | `// md:Access` | Access |
| 13 | `impl Access` | `// md:impl Access` | impl Access |
| 14 | `fn owner` | `// md:impl Access > fn owner` | impl Access › fn owner |
| 15 | `fn granted` | `// md:impl Access > fn granted` | impl Access › fn granted |
| 16 | `can_read`…`can_transfer_ownership` (5 fns) | `// md:impl Access > accessors` | impl Access › accessors |
| 17 | `fn resolve_note_access` | `// md:fn resolve_note_access` | fn resolve_note_access |
| 18 | `fn resolve_notebook_access` | `// md:fn resolve_notebook_access` | fn resolve_notebook_access |
| 19 | `mod tests` | `// md:mod tests` | mod tests |
| 20 | `fn higher_bits_imply_lower_ones` | `// md:mod tests > fn higher_bits_imply_lower_ones` | mod tests › fn higher_bits_imply_lower_ones |
| 21 | `fn read_alone_implies_nothing_more` | `// md:mod tests > fn read_alone_implies_nothing_more` | mod tests › fn read_alone_implies_nothing_more |
| 22 | `fn unknown_bits_are_masked_off` | `// md:mod tests > fn unknown_bits_are_masked_off` | mod tests › fn unknown_bits_are_masked_off |
| 23 | `fn owner_has_every_capability` | `// md:mod tests > fn owner_has_every_capability` | mod tests › fn owner_has_every_capability |
