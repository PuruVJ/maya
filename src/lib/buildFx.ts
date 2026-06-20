// A tiny channel from the edit tools to the dust particle system (SpawnPuffs). A tap-DELETE pushes a "poof
// here" so the object crumbles to dust as it vanishes — the mirror of the build pop-in's spawn dust, closing
// the "world reacts to your editing" loop. SpawnPuffs drains this each frame. Only explicit tap-deletes push
// (not bulk clear / LLM removes), so there's never a dust-storm.
export const deletePoofs: { x: number; z: number; r: number }[] = [];
