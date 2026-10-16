## ADDED Requirements

### Requirement: Room-based WebSocket signaling
The server SHALL expose `GET /ws?room=<id>` and group connections by room id in memory, with at most 2 peers per room.

#### Scenario: Second peer joins
- **WHEN** a second peer connects to a room with one peer
- **THEN** the first peer receives `PeerJoined` and the second receives `Joined{peer_count: 2}`

#### Scenario: Room full
- **WHEN** a third peer connects to a room with two peers
- **THEN** it receives `RoomFull` and its connection is closed

### Requirement: Opaque relay
The server SHALL forward `Offer`, `Answer` and `Ice` messages verbatim to the other peer in the room and MUST NOT persist them.

#### Scenario: Offer relayed
- **WHEN** peer B sends `Offer{sdp}`
- **THEN** peer A receives the identical `Offer{sdp}`

### Requirement: Peer departure
When a peer disconnects, the server SHALL notify the remaining peer with `PeerLeft` and SHALL drop the room once it is empty.

#### Scenario: Peer leaves
- **WHEN** one of two peers closes its socket
- **THEN** the other receives `PeerLeft`

### Requirement: Serves the web app
The server SHALL serve the built wasm app as static files, so one binary runs the demo.

#### Scenario: Load app
- **WHEN** a browser requests `/`
- **THEN** the app's `index.html` is returned
