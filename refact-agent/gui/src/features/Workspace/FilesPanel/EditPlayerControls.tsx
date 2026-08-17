import { Pause, Play, SkipForward, Square } from "lucide-react";

import { IconButton, Tooltip } from "../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import { nextEditPlayerSpeed } from "./editPlayer";
import {
  advanceEditPlayer,
  resetEditPlayer,
  selectEditPlayer,
  setEditPlayerSpeed,
  setEditPlayerStatus,
} from "./filesPanelSlice";
import styles from "./FilesPanel.module.css";

export function EditPlayerControls() {
  const dispatch = useAppDispatch();
  const player = useAppSelector(selectEditPlayer);

  if (player.status === "idle" || player.steps.length === 0) return null;

  const playing = player.status === "playing";
  const position = Math.min(player.index + 1, player.steps.length);

  return (
    <div
      aria-label="Edit player"
      className={styles.playerControls}
      role="group"
    >
      <Tooltip
        content={playing ? "Pause edit playback" : "Resume edit playback"}
      >
        <IconButton
          aria-label={playing ? "Pause edit playback" : "Resume edit playback"}
          icon={playing ? Pause : Play}
          onClick={() =>
            dispatch(setEditPlayerStatus(playing ? "paused" : "playing"))
          }
          size="sm"
          variant="plain"
        />
      </Tooltip>
      <Tooltip content="Skip to next edit">
        <IconButton
          aria-label="Skip to next edit"
          disabled={player.index >= player.steps.length}
          icon={SkipForward}
          onClick={() => dispatch(advanceEditPlayer())}
          size="sm"
          variant="plain"
        />
      </Tooltip>
      <span className={styles.playerPosition}>
        {position} / {player.steps.length}
      </span>
      <button
        aria-label={`Playback speed ${player.speed}x`}
        className={styles.playerSpeed}
        onClick={() =>
          dispatch(setEditPlayerSpeed(nextEditPlayerSpeed(player.speed)))
        }
        type="button"
      >
        {player.speed}x
      </button>
      <Tooltip content="Stop edit playback">
        <IconButton
          aria-label="Stop edit playback"
          icon={Square}
          onClick={() => dispatch(resetEditPlayer())}
          size="sm"
          variant="plain"
        />
      </Tooltip>
    </div>
  );
}
