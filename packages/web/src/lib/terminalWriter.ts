/** xterm.write is asynchronous; reset/resize must wait for earlier writes.
 * A checkpoint supersedes queued drawing, but never interrupts a partial write. */
export class TerminalWriter {
  private queue: Array<() => void> = [];
  private writing = false;
  private disposed = false;
  private pendingBytes = 0;
  private resetVersion = 0;
  private awaitingRestore = false;
  get restoring() {
    return this.awaitingRestore;
  }
  constructor(
    private readonly terminal: {
      write(bytes: Uint8Array, done: () => void): void;
      reset(): void;
      resize(cols: number, rows: number): void;
    },
    private readonly onOverflow: () => void,
  ) {}
  private next = () => {
    this.writing = false;
    if (this.disposed) return;
    const operation = this.queue.shift();
    if (operation) {
      this.writing = true;
      operation();
    }
  };
  private schedule(operation: () => void) {
    this.queue.push(operation);
    if (!this.writing) this.next();
  }
  write(bytes: Uint8Array) {
    if (this.pendingBytes + bytes.length > 20 * 1024 * 1024) {
      this.queue = [];
      this.pendingBytes = 0;
      this.awaitingRestore = true;
      this.resetVersion++;
      this.onOverflow();
      return;
    }
    const version = this.resetVersion;
    const restores = this.awaitingRestore;
    this.pendingBytes += bytes.length;
    this.schedule(() => {
      this.pendingBytes -= bytes.length;
      this.terminal.write(bytes, () => {
        if (restores && version === this.resetVersion)
          this.awaitingRestore = false;
        this.next();
      });
    });
  }
  reset() {
    this.resetVersion++;
    this.awaitingRestore = true;
    this.queue = [];
    this.pendingBytes = 0;
    this.schedule(() => {
      this.terminal.reset();
      this.next();
    });
  }
  resize(cols: number, rows: number) {
    this.schedule(() => {
      this.terminal.resize(cols, rows);
      this.next();
    });
  }
  dispose() {
    this.disposed = true;
    this.queue = [];
  }
}
