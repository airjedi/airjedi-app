import { Aircraft } from "./types";

type ChangeCallback = (aircraft: Map<string, Aircraft>) => void;
type RemoveCallback = (icao: string[]) => void;

export class AircraftStore {
  private aircraft = new Map<string, Aircraft>();
  private changeCallbacks: ChangeCallback[] = [];
  private removeCallbacks: RemoveCallback[] = [];

  applySnapshot(list: Aircraft[]): void {
    this.aircraft.clear();
    for (const ac of list) {
      this.aircraft.set(ac.icao, ac);
    }
    this.notifyChange();
  }

  applyUpdate(list: Aircraft[]): void {
    for (const ac of list) {
      const existing = this.aircraft.get(ac.icao);
      if (existing && ac.trail.length > 0) {
        existing.trail.push(...ac.trail);
        if (existing.trail.length > 200) {
          existing.trail = existing.trail.slice(-200);
        }
        Object.assign(existing, { ...ac, trail: existing.trail });
      } else {
        this.aircraft.set(ac.icao, ac);
      }
    }
    this.notifyChange();
  }

  applyRemove(icao: string[]): void {
    for (const id of icao) {
      this.aircraft.delete(id);
    }
    for (const cb of this.removeCallbacks) cb(icao);
    this.notifyChange();
  }

  getAll(): Map<string, Aircraft> {
    return this.aircraft;
  }

  get(icao: string): Aircraft | undefined {
    return this.aircraft.get(icao);
  }

  get count(): number {
    return this.aircraft.size;
  }

  onChange(cb: ChangeCallback): void {
    this.changeCallbacks.push(cb);
  }

  onRemove(cb: RemoveCallback): void {
    this.removeCallbacks.push(cb);
  }

  private notifyChange(): void {
    for (const cb of this.changeCallbacks) cb(this.aircraft);
  }
}

export class AppState {
  selectedIcao: string | null = null;
  private selectionCallbacks: ((icao: string | null) => void)[] = [];

  select(icao: string | null): void {
    this.selectedIcao = icao;
    for (const cb of this.selectionCallbacks) cb(icao);
  }

  onSelectionChange(cb: (icao: string | null) => void): void {
    this.selectionCallbacks.push(cb);
  }
}
