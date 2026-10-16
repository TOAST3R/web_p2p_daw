# session-view Specification

## Purpose
TBD - created by archiving change foundation-session-view. Update Purpose after archive.
## Requirements
### Requirement: Session grid
The UI SHALL show tracks as columns and 8 scenes as rows of clip slots, with a stop button, volume control and mute toggle per track, a launch button per scene, and buttons to add Synth and Drum tracks.

#### Scenario: Create a clip
- **WHEN** the user double-clicks an empty slot
- **THEN** a 1-bar clip with no notes is created in that slot and selected

#### Scenario: Delete a track
- **WHEN** the user deletes a track
- **THEN** its column disappears on both peers

### Requirement: Quantized clip launch
Launching a clip SHALL take effect at the next bar boundary (4 beats), and the launch SHALL be stored in shared state so all peers start it on the same bar. If the transport is stopped, launching SHALL start the transport and the clip at beat 0.

#### Scenario: Launch while playing
- **WHEN** a clip is launched at beat 9.3
- **THEN** it starts at beat 12 on both peers, and the previous clip on that track plays until beat 12

#### Scenario: Launch while stopped
- **WHEN** a clip is launched while the transport is stopped
- **THEN** the transport starts and the clip plays from beat 0

#### Scenario: Concurrent launches on one track
- **WHEN** both peers launch different clips on the same track before the next bar
- **THEN** both peers end up playing the same clip

### Requirement: Track stop
The track stop button SHALL stop the track's clip at the next bar boundary.

#### Scenario: Stop a track
- **WHEN** the stop button is pressed at beat 5
- **THEN** the track falls silent at beat 8

### Requirement: Scene launch
Launching a scene SHALL launch, in one transaction, the clip in that row for every track. Tracks with an empty slot in that row SHALL stop.

#### Scenario: Launch scene 2
- **WHEN** scene 2 is launched and track B has no clip in row 2
- **THEN** all other tracks switch to their row-2 clips at the next bar and track B stops

### Requirement: Clip state display
A clip slot SHALL show whether its clip is stopped, queued (blinking until its start bar), or playing (with loop progress).

#### Scenario: Queued then playing
- **WHEN** a clip is launched at beat 2
- **THEN** it shows as queued until beat 4 and as playing from beat 4

