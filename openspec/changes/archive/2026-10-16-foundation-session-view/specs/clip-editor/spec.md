## ADDED Requirements

### Requirement: Piano roll for the selected clip
The detail panel SHALL show the selected clip's notes on a time × pitch grid that snaps to 1/16 notes. Synth tracks SHALL show pitch rows, and Drum tracks SHALL show 4 named lanes (Kick 36, Snare 38, Hat 42, Bass 45).

#### Scenario: Add a note
- **WHEN** the user clicks an empty cell at beat 1.25, pitch 60
- **THEN** a note with start 1.25, pitch 60, length 0.25 and velocity 0.8 is added to the clip

#### Scenario: Move and resize
- **WHEN** the user drags a note's body by one 1/16 to the right, then drags its right edge one 1/16 further
- **THEN** its start increases by 0.25 and its length increases by 0.25

#### Scenario: Delete
- **WHEN** the user right-clicks a note
- **THEN** the note is removed

### Requirement: Velocity editing
The panel SHALL show a velocity strip under the grid where dragging a note's bar sets its velocity between 0 and 1.

#### Scenario: Lower velocity
- **WHEN** the user drags a note's velocity bar to half height
- **THEN** the note's velocity becomes 0.5

### Requirement: Clip length
The user SHALL be able to set the selected clip's length to 1, 2, 4 or 8 bars. Notes beyond the length SHALL be kept but not played.

#### Scenario: Shorten a clip
- **WHEN** a 2-bar clip with a note at beat 6 is set to 1 bar
- **THEN** the note remains in the clip but is not played

### Requirement: Synth parameters
For Synth tracks, the panel SHALL show controls for wave (saw/square), cutoff, resonance, attack, decay, sustain and release, stored in the shared track params.

#### Scenario: Remote parameter change is heard
- **WHEN** one peer lowers the cutoff
- **THEN** the other peer's audio for that track changes accordingly

### Requirement: Edits are live and shared
Every piano roll and parameter edit SHALL be written to the shared document immediately, and SHALL be heard on the next loop pass without stopping playback.

#### Scenario: Edit while playing
- **WHEN** a note is added ahead of the playhead in a playing clip
- **THEN** it sounds on both peers when the playhead reaches it
