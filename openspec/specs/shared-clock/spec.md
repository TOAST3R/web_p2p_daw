# shared-clock Specification

## Purpose
TBD - created by archiving change foundation-session-view. Update Purpose after archive.
## Requirements
### Requirement: Clock offset estimation
Peers SHALL exchange Ping/Pong messages over the data channel. The peer with the higher Yrs client id SHALL estimate its offset to the other (the reference) from the sample with the lowest round-trip time among the last 8. The reference's offset SHALL be 0.

#### Scenario: Offset from symmetric samples
- **WHEN** a ping is sent at local 1000, received by the reference at its time 5050, and the pong returns at local 1100
- **THEN** the estimated offset is 4000 ms

#### Scenario: High-latency samples are ignored
- **WHEN** one sample has a 300 ms round trip and another has a 20 ms round trip
- **THEN** the offset comes from the 20 ms sample

#### Scenario: Ping schedule
- **WHEN** the data channel opens
- **THEN** 5 pings are sent 100 ms apart and then one every second

### Requirement: Shared transport timeline
The transport SHALL be defined by `playing`, `anchor_ms` (shared time), `anchor_beat` and `bpm`, and every peer SHALL compute the beat as `anchor_beat + (shared_now − anchor_ms) · bpm / 60000`.

#### Scenario: Same beat on both peers
- **WHEN** both peers have a converged offset and the transport is playing
- **THEN** their computed beat positions differ by less than 10 ms worth of beats

#### Scenario: Play starts in the near future
- **WHEN** a user presses play
- **THEN** `anchor_beat` is 0 and `anchor_ms` is 300 ms after the current shared time

#### Scenario: BPM change keeps position
- **WHEN** the BPM changes from 120 to 90 at beat 17.5 while playing
- **THEN** the beat is still 17.5 immediately after the change and advances at 90 BPM

#### Scenario: Stop
- **WHEN** a user presses stop
- **THEN** both peers stop and all track launches are cleared

### Requirement: Output latency compensation
The engine SHALL map audio frames to shared time using the audio context's output timestamp, re-anchored at least every second, plus a per-user latency nudge between −200 and +200 ms that is not synced.

#### Scenario: Latency nudge is local
- **WHEN** a user sets the nudge to +40 ms
- **THEN** only that user's audio shifts later by 40 ms and the other peer is unaffected

### Requirement: Reference handover
When the reference peer disconnects, the remaining peer SHALL become the reference, keep its current offset (so its timeline does not jump), and keep the transport playing. Pongs SHALL carry the responder's shared time.

#### Scenario: Reference leaves
- **WHEN** the reference peer closes its tab while playing
- **THEN** the other peer keeps playing without stopping

