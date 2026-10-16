## ADDED Requirements

### Requirement: Rust DSP runs inside an AudioWorklet
The system SHALL render audio by running the Rust `dsp` crate compiled to wasm inside a browser AudioWorklet, with no audio processing on the main thread.

#### Scenario: Engine produces sound
- **WHEN** the user clicks "Start audio" and a pattern with at least one active cell is playing
- **THEN** audible output is produced by the worklet's wasm `process` function

#### Scenario: No glitches under UI load
- **WHEN** the pattern plays for 60 seconds while the user continuously interacts with the UI
- **THEN** no audible dropouts occur in Chrome on a typical laptop

### Requirement: Audio starts on user gesture
The system SHALL create the AudioContext and load the worklet only in response to a user click.

#### Scenario: Before click
- **WHEN** the page loads
- **THEN** no AudioContext is running and a "Start audio" control is shown

### Requirement: Sample-accurate local step clock
The engine SHALL advance sequencer steps based on the sample count at the current BPM (16th notes) inside the worklet.

#### Scenario: Step timing
- **WHEN** BPM is 120 at sample rate 48000
- **THEN** each step lasts exactly 6000 samples

### Requirement: Local transport
The engine SHALL support play and stop controlled locally. Play SHALL start from step 0.

#### Scenario: Stop silences output
- **WHEN** the user presses stop
- **THEN** no new notes are triggered and the step position resets to 0

### Requirement: Live pattern updates
The engine SHALL accept a new pattern snapshot at any time and use it from the next step onward without restarting playback.

#### Scenario: Toggling while playing
- **WHEN** a cell is enabled while the transport is playing
- **THEN** that cell sounds the next time the playhead reaches it
