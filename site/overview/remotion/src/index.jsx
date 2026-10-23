import { Composition, registerRoot } from "remotion";
import { Overview } from "./Overview.jsx";

// The timeline comes in as props, written by ../make.mjs: scene timings
// follow the narration, so nothing here hard-codes a length.
const Root = () => (
  <Composition
    id="Overview"
    component={Overview}
    width={1920}
    height={1080}
    fps={30}
    durationInFrames={300}
    defaultProps={{ width: 1920, height: 1080, fps: 30, frames: 300, fade: 12, scenes: [], music: null }}
    calculateMetadata={({ props }) => ({
      durationInFrames: props.frames,
      fps: props.fps,
      width: props.width,
      height: props.height,
    })}
  />
);

registerRoot(Root);
