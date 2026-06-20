// Native-eval step 1: emit the scenario cases as the FINE-TUNE actually sees them (compact world-state
// system prompt). Reuses the real SCENARIOS + buildWorldState — no duplication. → eval-cases.jsonl
import { writeFileSync } from 'node:fs';
import { buildWorldState } from '../src/lib/llm-prompt';
import { demoWorld, type Player } from '../src/lib/world';
import { SCENARIOS } from '../tests/llm/scenarios';

const PLAYER: Player = { pos: [0, 0, 6], yaw: 0 };
const lines = SCENARIOS.map((s, i) =>
	JSON.stringify({ i, system: buildWorldState(s.world ?? demoWorld(), PLAYER), prompt: s.t })
);
writeFileSync('training/data/eval-cases.jsonl', lines.join('\n') + '\n');
console.log(`✓ wrote ${lines.length} eval cases → training/data/eval-cases.jsonl`);
