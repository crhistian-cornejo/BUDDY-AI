/** Track screen coordinates: resizing the native window changes client coordinates during a press. */
export class PetPress {
  private point: { id: number; x: number; y: number } | null = null;
  private dragged = false;

  get active() { return this.point !== null; }
  hasPointer(id: number) { return this.point?.id === id; }

  start(id: number, x: number, y: number) {
    this.point = { id, x, y };
    this.dragged = false;
  }

  move(id: number, x: number, y: number) {
    if (!this.point || this.point.id !== id || Math.hypot(x - this.point.x, y - this.point.y) <= 4) return false;
    return this.hold();
  }

  hold() {
    if (!this.point || this.dragged) return false;
    this.dragged = true;
    return true;
  }

  release(id: number): "click" | "drag" | "none" {
    if (!this.hasPointer(id)) return "none";
    const result = this.dragged ? "drag" : "click";
    this.cancel();
    return result;
  }

  cancel() { this.point = null; this.dragged = false; }
}
