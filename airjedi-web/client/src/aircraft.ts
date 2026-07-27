import {
  Viewer,
  Entity,
  Cartesian3,
  Cartesian2,
  Color,
  VerticalOrigin,
  HorizontalOrigin,
  NearFarScalar,
  PolylineGlowMaterialProperty,
  ScreenSpaceEventHandler,
  ScreenSpaceEventType,
  defined,
  LabelStyle,
  ConstantPositionProperty,
  ConstantProperty,
} from "cesium";
import { Aircraft } from "./types";
import { AircraftStore, AppState } from "./store";
import { formatAlt } from "./theme";

function altitudeColor(altFeet: number | null): Color {
  if (altFeet === null) return Color.fromCssColorString("#646464");
  if (altFeet >= 30000) return Color.fromCssColorString("#c864ff");
  if (altFeet >= 20000) return Color.fromCssColorString("#ff9632");
  if (altFeet >= 10000) return Color.fromCssColorString("#c8c864");
  return Color.fromCssColorString("#64c8c8");
}

function feetToMeters(feet: number): number {
  return feet * 0.3048;
}

function formatAltitude(alt: number | null): string {
  return formatAlt(alt);
}

export class AircraftManager {
  private entities = new Map<string, Entity>();
  private viewer: Viewer;
  private store: AircraftStore;
  private appState: AppState;

  constructor(viewer: Viewer, store: AircraftStore, appState: AppState) {
    this.viewer = viewer;
    this.store = store;
    this.appState = appState;

    store.onChange(() => this.syncEntities());
    store.onRemove((icao) => this.removeEntities(icao));

    this.setupPicking();
  }

  private syncEntities(): void {
    const aircraft = this.store.getAll();

    for (const [icao, ac] of aircraft) {
      if (ac.latitude === null || ac.longitude === null) continue;

      const existing = this.entities.get(icao);
      if (existing) {
        this.updateEntity(existing, ac);
      } else {
        this.createEntity(ac);
      }
    }

    const stale: string[] = [];
    for (const icao of this.entities.keys()) {
      if (!aircraft.has(icao)) {
        stale.push(icao);
      }
    }
    if (stale.length > 0) {
      this.removeEntities(stale);
    }
  }

  private createEntity(ac: Aircraft): void {
    if (ac.latitude === null || ac.longitude === null) return;

    const altMeters = ac.altitude ? feetToMeters(ac.altitude) : 0;
    const color = altitudeColor(ac.altitude);
    const label = ac.callsign || ac.icao;

    const entity = this.viewer.entities.add({
      id: `aircraft-${ac.icao}`,
      position: Cartesian3.fromDegrees(ac.longitude, ac.latitude, altMeters),
      point: {
        pixelSize: 8,
        color: color,
        outlineColor: Color.BLACK,
        outlineWidth: 1,
      },
      label: {
        text: `${label}\n${formatAltitude(ac.altitude)}`,
        font: "11px sans-serif",
        fillColor: Color.WHITE,
        outlineColor: Color.BLACK,
        outlineWidth: 2,
        style: LabelStyle.FILL_AND_OUTLINE,
        verticalOrigin: VerticalOrigin.BOTTOM,
        horizontalOrigin: HorizontalOrigin.LEFT,
        pixelOffset: new Cartesian2(10, -5),
        scaleByDistance: new NearFarScalar(5e4, 1.0, 5e6, 0.3),
        translucencyByDistance: new NearFarScalar(5e5, 1.0, 1e7, 0.0),
      },
    });

    if (ac.trail.length > 1) {
      this.updateTrail(ac);
    }

    this.entities.set(ac.icao, entity);
  }

  private updateEntity(entity: Entity, ac: Aircraft): void {
    if (ac.latitude === null || ac.longitude === null) return;

    const altMeters = ac.altitude ? feetToMeters(ac.altitude) : 0;
    entity.position = new ConstantPositionProperty(
      Cartesian3.fromDegrees(ac.longitude, ac.latitude, altMeters)
    );

    if (entity.point) {
      entity.point.color = new ConstantProperty(altitudeColor(ac.altitude));
    }

    if (entity.label) {
      const label = ac.callsign || ac.icao;
      entity.label.text = new ConstantProperty(
        `${label}\n${formatAltitude(ac.altitude)}`
      );
    }

    if (ac.trail.length > 1) {
      this.updateTrail(ac);
    }
  }

  private updateTrail(ac: Aircraft): void {
    const trailId = `trail-${ac.icao}`;
    const trailEntity = this.viewer.entities.getById(trailId);

    const positions = ac.trail
      .filter((p) => p.lat !== null && p.lon !== null)
      .map((p) =>
        Cartesian3.fromDegrees(
          p.lon,
          p.lat,
          p.alt ? feetToMeters(p.alt) : 0
        )
      );

    if (ac.latitude !== null && ac.longitude !== null) {
      const altMeters = ac.altitude ? feetToMeters(ac.altitude) : 0;
      positions.push(
        Cartesian3.fromDegrees(ac.longitude, ac.latitude, altMeters)
      );
    }

    if (positions.length < 2) return;

    if (trailEntity) {
      if (trailEntity.polyline) {
        trailEntity.polyline.positions = new ConstantProperty(positions);
      }
    } else {
      this.viewer.entities.add({
        id: trailId,
        polyline: {
          positions: positions,
          width: 2,
          material: new PolylineGlowMaterialProperty({
            glowPower: 0.2,
            color: altitudeColor(ac.altitude),
          }),
          clampToGround: false,
        },
      });
    }
  }

  private removeEntities(icao: string[]): void {
    for (const id of icao) {
      const entity = this.entities.get(id);
      if (entity) {
        this.viewer.entities.remove(entity);
        this.entities.delete(id);
      }
      const trailEntity = this.viewer.entities.getById(`trail-${id}`);
      if (trailEntity) {
        this.viewer.entities.remove(trailEntity);
      }
    }
  }

  private setupPicking(): void {
    const handler = new ScreenSpaceEventHandler(this.viewer.scene.canvas);
    handler.setInputAction((event: { position: Cartesian2 }) => {
      const picked = this.viewer.scene.pick(event.position);
      if (defined(picked) && picked.id?.id?.startsWith("aircraft-")) {
        const icao = picked.id.id.replace("aircraft-", "");
        this.appState.select(icao);
      } else {
        this.appState.select(null);
      }
    }, ScreenSpaceEventType.LEFT_CLICK);
  }
}
