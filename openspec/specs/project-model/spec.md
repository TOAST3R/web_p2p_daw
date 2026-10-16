# project-model Specification

## Purpose
TBD - created by archiving change foundation-session-view. Update Purpose after archive.
## Requirements
### Requirement: Project schema in Yrs
The project SHALL be stored in a Yrs document with root maps `transport`, `tracks` and `clips`, where tracks and clips are keyed by id, each track has 8 clip slots, and each clip holds notes keyed by id.

#### Scenario: Empty project
- **WHEN** a new document is created
- **THEN** it contains no tracks and no clips, the transport is stopped, and BPM reads as 120

#### Scenario: Add a track
- **WHEN** the user adds a Synth track
- **THEN** a track entry with kind `synth`, default synth params, volume 0.8, mute false and 8 empty slots exists in `tracks`

### Requirement: Collision-free ids
Track, clip and note ids SHALL be generated as `{yrs_client_id}-{counter}` so two peers never generate the same id.

#### Scenario: Concurrent creation
- **WHEN** both peers add a track at the same moment
- **THEN** after sync the project has two distinct tracks

### Requirement: Stable track order
Tracks SHALL be ordered by a fractional `order` value, with ties broken by id, so concurrent additions and deletions never reorder other tracks.

#### Scenario: Concurrent add keeps a deterministic order
- **WHEN** both peers add a track concurrently and both get the same `order`
- **THEN** both peers display the tracks in the same order

### Requirement: Deleting a track removes its clips
Deleting a track SHALL remove the track and the clips referenced by its slots in a single transaction.

#### Scenario: Delete track
- **WHEN** a track with two clips is deleted
- **THEN** neither the track nor its two clips remain in the document

### Requirement: Engine state derivation
The app SHALL derive a serialized engine state (transport, tracks, instruments, effective and previous clips with notes) from the document whenever the document changes, and SHALL send it to the audio engine only when the bytes differ from the last send.

#### Scenario: Remote edit reaches the engine
- **WHEN** a remote peer adds a note to a playing clip
- **THEN** the local engine receives a new state containing that note

