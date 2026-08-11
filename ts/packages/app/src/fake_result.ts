// Ported from game/fake_result.lua.
//
// The Lua original reads `data.players` directly and calls
// `game.match_contract.new_result` (which reads `data.teams` itself). Both
// tables are Rust-owned (v2/README.md rule 6.7), so `forRequest` takes the
// slice of `MatchContractContent` it needs -- `players` for the MVP lookup,
// `teams` because `matchContract.newResult` requires it -- as an explicit
// parameter.

import { matchContract, type ProductMatchRequest, type ProductMatchResult } from "./match_contract.ts";
import type { MatchContractContent } from "./content.ts";

function hash(value: string): number {
  let result = 17;
  for (let i = 0; i < value.length; i += 1) {
    result = (result * 31 + value.charCodeAt(i)) % 104729;
  }
  return result;
}

export function forRequest(
  content: Pick<MatchContractContent, "players" | "teams">,
  request: ProductMatchRequest,
): ProductMatchResult {
  const seed = request.seed ?? 1;
  const value = hash(
    [request.formation_id, request.tactic_id, String(seed), request.home_starter_ids.join(",")].join(":"),
  );
  const homeScore = value % 4;
  const awayScore = Math.floor(value / 7) % 4;
  const homePossession = 0.42 + (value % 17) / 100;
  let mvp = request.home_starter_ids[1];
  for (const id of request.home_starter_ids) {
    const player = content.players[id];
    if (player && player.position !== "keeper") {
      mvp = id;
      break;
    }
  }

  const result = matchContract.newResult(content, {
    home_team_id: request.home_team_id,
    away_team_id: request.away_team_id,
    home_score: homeScore,
    away_score: awayScore,
    ...(mvp !== undefined ? { mvp_player_id: mvp } : {}),
    mvp_summary: "Drove the showcase plan from first whistle to full time.",
    home_stats: {
      shots: 6 + (value % 7),
      possession: homePossession,
      saves: 1 + (value % 4),
      pass_completion: 0.58 + (value % 20) / 100,
    },
    away_stats: {
      shots: 5 + (Math.floor(value / 3) % 7),
      possession: 1 - homePossession,
      saves: 1 + (Math.floor(value / 5) % 4),
      pass_completion: 0.52 + (value % 18) / 100,
    },
    seed,
  });
  if (!result.ok) {
    throw new Error(result.error);
  }
  return result.value;
}

export const fakeResult = { forRequest };
