# step-sequencer Specification

## Purpose
TBD - created by archiving change p2p-sequencer-mvp. Update Purpose after archive.
## Requirements
### Requirement: Pattern data model in Yrs
The pattern SHALL be stored in a Yrs document as a root map `pattern` containing `bpm` (number) and `cell:{track}:{step}` (bool) for 4 tracks × 16 steps.

#### Scenario: Default pattern
- **WHEN** a new document is created
- **THEN** BPM reads as 120 and all cells read as off

### Requirement: Grid UI in egui
The system SHALL render a 4 × 16 grid of toggle cells, a BPM control, play/stop buttons, and a moving playhead indicator using egui.

#### Scenario: Toggle a cell
- **WHEN** the user clicks an off cell
- **THEN** the cell is set to on in the Yrs document and is shown as on

#### Scenario: Change BPM
- **WHEN** the user sets BPM to 90
- **THEN** `bpm` in the document becomes 90 and playback tempo changes accordingly

### Requirement: UI reflects remote changes
The UI SHALL re-render from the Yrs document whenever a remote update is applied.

#### Scenario: Remote toggle appears
- **WHEN** a remote update turns on cell (2, 5)
- **THEN** that cell is shown as on without any local interaction

### Requirement: Pattern changes reach the audio engine
After any local or remote change, the system SHALL derive a pattern snapshot from the document and send it to the audio engine.

#### Scenario: Remote change is heard
- **WHEN** a remote peer enables a cell while local playback runs
- **THEN** the local engine plays that cell on the next pass

