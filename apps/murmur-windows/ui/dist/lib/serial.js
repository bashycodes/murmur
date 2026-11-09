// One queue for a settings page's writes: each runs after the one before
// has finished (or failed), so a reload never reads the settings while an
// earlier save is still being written. No DOM: unit-tested under ui/tests.
export class Serial {
    constructor() {
        this.tail = Promise.resolve();
    }
    /** Run `job` after every job queued before it. */
    run(job) {
        const next = this.tail.then(job, job);
        this.tail = next.catch(() => undefined);
        return next;
    }
}
