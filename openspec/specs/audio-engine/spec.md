# audio-engine Specification

## Purpose
TBD - created by archiving change p2p-sequencer-mvp. Update Purpose after archive.
## Requirements
### Requirement: Rust DSP runs inside an AudioWorklet
The system SHALL render audio by running the Rust `dsp` crate compiled to wasm inside a browser AudioWorklet, with no audio processing on the main thread.

#### Scenario: Engine produces sound
- **WHEN** the user clicks "Start audio" and a clip with at least one note is playing
- **THEN** audible output is produced by the worklet's wasm `process` function

#### Scenario: No glitches under UI load
- **WHEN** a session with 4 tracks plays for 60 seconds while the user continuously edits in the piano roll
- **THEN** no audible dropouts occur in Chrome on a typical laptop

### Requirement: Audio starts on user gesture
The system SHALL create the AudioContext and load the worklet only in response to a user click.

#### Scenario: Before click
- **WHEN** the page loads
- **THEN** no AudioContext is running and a "Start audio" control is shown

### Requirement: Sample-accurate scheduling from the shared timeline
The engine SHALL compute the beat range of each render block from the frame-to-shared-time anchor and the transport, and SHALL start and end notes at their exact frame offsets, including across clip loop boundaries.

#### Scenario: Note on a loop boundary
- **WHEN** a 1-bar clip at 120 BPM and 48 kHz has a note at beat 0 and is playing
- **THEN** the note starts at frames that are exact multiples of 96000 after the clip's start

### Requirement: Multi-track instruments
The engine SHALL render every unmuted track's instrument, scaled by track volume, into a mono master clamped to [−1, 1]. Instruments SHALL be an 8-voice Synth (saw/square → resonant low-pass → ADSR, oldest-voice stealing) and a Drum Kit (kick 36, snare 38, hat 42, bass 45).

#### Scenario: Muted track is silent
- **WHEN** a track is muted
- **THEN** it contributes nothing to the output

#### Scenario: Voice stealing
- **WHEN** a Synth track plays a 9th simultaneous note
- **THEN** the oldest sounding voice is reused

### Requirement: Engine state updates without interruption
The engine SHALL accept a new serialized state at any time and keep voices that are already sounding, matched by track id.

#### Scenario: Edit while a note sounds
- **WHEN** a new state arrives while a long synth note is sounding on that track
- **THEN** the note continues without a click

### Requirement: Malformed state is rejected
The engine SHALL reject state bytes that fail to decode, and SHALL keep playing the previous state.

#### Scenario: Bad bytes
- **WHEN** `load_state` receives corrupt bytes
- **THEN** it returns an error code and playback continues unchanged

