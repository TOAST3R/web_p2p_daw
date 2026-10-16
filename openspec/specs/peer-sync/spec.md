# peer-sync Specification

## Purpose
TBD - created by archiving change p2p-sequencer-mvp. Update Purpose after archive.
## Requirements
### Requirement: Direct WebRTC data channel
Two peers in the same room SHALL connect via an `RtcPeerConnection` using public STUN and exchange pattern data only over a reliable, ordered data channel labeled `yrs`. Pattern data MUST NOT pass through the signaling server.

#### Scenario: Connection established
- **WHEN** two browsers open the same room URL
- **THEN** a data channel opens between them and both UIs show "connected"

#### Scenario: Server never sees pattern data
- **WHEN** peers edit the pattern after connecting
- **THEN** the signaling server receives no messages containing Yrs updates

### Requirement: Initial state exchange
On channel open, each peer SHALL send its state vector (SyncStep1) and reply to the other's with the missing diff (SyncStep2).

#### Scenario: Late joiner catches up
- **WHEN** peer A has edited the pattern and peer B then connects with an empty document
- **THEN** B's document converges to A's pattern

#### Scenario: Both had offline edits
- **WHEN** both peers edited different cells before connecting
- **THEN** after sync both documents contain all edits

### Requirement: Incremental updates
Each local change SHALL be broadcast as an Update message. Updates received from the network MUST NOT be re-broadcast.

#### Scenario: Live edit propagates
- **WHEN** peer A toggles a cell
- **THEN** peer B's document reflects it within one network round trip

#### Scenario: No echo
- **WHEN** peer B applies an update received from A
- **THEN** B sends no Update message for it

### Requirement: Convergence under concurrent edits
Concurrent edits SHALL converge to identical documents on both peers.

#### Scenario: Same cell toggled concurrently
- **WHEN** A and B set the same cell to different values at the same time
- **THEN** both documents end with the same value for that cell

### Requirement: Disconnect handling
The system SHALL keep the local document editable when the peer disconnects, and SHALL resync via the initial exchange when a peer reconnects.

#### Scenario: Reconnect after offline edits
- **WHEN** the peer leaves, local edits are made, and the peer rejoins the room
- **THEN** both documents converge including the edits made while disconnected

