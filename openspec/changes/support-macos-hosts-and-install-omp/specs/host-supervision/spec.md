## ADDED Requirements

### Requirement: One daemon per user and host on every platform
The CLI SHALL decide whether a daemon is already running without relying on a Linux-only process filesystem, and a daemon SHALL hold an exclusive per-user lock for its lifetime that a requested restart keeps under the same process. A second daemon that cannot take the lock SHALL say which daemon is running and exit. A daemon that finds other daemons of the same user that hold no lock SHALL name them and SHALL NOT stop them.

#### Scenario: A second daemon on macOS
- **WHEN** a daemon is running on a Mac and the person runs `apas daemon` or `apas` in a project directory
- **THEN** no second daemon is started, and `apas daemon` prints the running daemon's pid and version

#### Scenario: Daemons from an older APAS
- **WHEN** a new daemon starts while daemons of an older APAS of the same user run on the host
- **THEN** it prints and logs their pids with how to stop them, and keeps running without stopping them

### Requirement: macOS daemon state survives launch contexts
On macOS without `XDG_RUNTIME_DIR`, the daemon's state SHALL live in one per-user directory that is the same for every launch context and that the system does not clean. The first daemon of a new version SHALL still find the projects to resume and the pane hosts that the previous version recorded in its temporary directory, and SHALL delete nothing there.

#### Scenario: GUI and ssh agree
- **WHEN** a daemon was started from a Terminal session and `apas` is later run over ssh
- **THEN** both read the same daemon state and no second daemon starts
