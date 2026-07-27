import {
  Viewer,
  Ion,
  OpenStreetMapImageryProvider,
  ImageryLayer,
  SceneMode,
  Cartesian3,
} from "cesium";

export async function initGlobe(
  container: HTMLElement,
  ionToken: string | null
): Promise<Viewer> {
  if (ionToken) {
    Ion.defaultAccessToken = ionToken;
  }

  const viewer = new Viewer(container, {
    sceneMode: SceneMode.SCENE3D,
    animation: false,
    timeline: false,
    homeButton: true,
    sceneModePicker: true,
    baseLayerPicker: true,
    navigationHelpButton: false,
    fullscreenButton: false,
    geocoder: false,
    infoBox: false,
    selectionIndicator: false,
  });

  const osmProvider = new OpenStreetMapImageryProvider({
    url: "https://tile.openstreetmap.org/",
  });
  const osmLayer = new ImageryLayer(osmProvider);
  osmLayer.show = false;
  viewer.imageryLayers.add(osmLayer);

  viewer.camera.setView({
    destination: Cartesian3.fromDegrees(-97.3301, 37.6872, 500000),
  });

  return viewer;
}
