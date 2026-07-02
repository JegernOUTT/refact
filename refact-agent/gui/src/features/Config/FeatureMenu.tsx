import React, { useState, useEffect, useCallback } from "react";
import {
  useAppDispatch,
  useAppSelector,
  useEventsBusForIDE,
} from "../../hooks";
import { selectFeatures, changeFeature } from "./configSlice";
import { Link } from "../../components/Link";
import { Button, Dialog, FieldSwitch, SettingItem } from "../../components/ui";
import styles from "./FeatureMenu.module.css";

const useInputEvent = () => {
  const [key, setKey] = useState<string | null>(null);
  useEffect(() => {
    const keyDownHandler = (event: KeyboardEvent) => setKey(event.code);
    const keyUpHandler = () => setKey(null);
    window.addEventListener("keydown", keyDownHandler);
    window.addEventListener("keyup", keyUpHandler);
    return () => {
      window.removeEventListener("keydown", keyDownHandler);
      window.removeEventListener("keyup", keyUpHandler);
    };
  }, []);

  return key;
};

const konamiCode = [
  "ArrowUp",
  "ArrowUp",
  "ArrowDown",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "ArrowLeft",
  "ArrowRight",
  "Escape",
  "Enter",
];

const useKonamiCode = () => {
  const [count, setCount] = useState(0);
  const [success, setSuccess] = useState(false);
  const key = useInputEvent();

  useEffect(() => {
    if (success) {
      return;
    } else if (document.activeElement !== document.body) {
      return;
    } else if (count === konamiCode.length) {
      setSuccess(true);
    } else if (key === konamiCode[count]) {
      setCount((n) => n + 1);
    }
  }, [key, count, success]);

  const reset = useCallback(() => {
    setSuccess(false);
    setCount(0);
  }, []);

  return { success, reset };
};

export const FeatureMenu: React.FC = () => {
  const { success, reset } = useKonamiCode();
  const dispatch = useAppDispatch();
  const features = useAppSelector(selectFeatures);

  const { openSettings } = useEventsBusForIDE();

  const handleSettingsClick = useCallback(
    (event: React.MouseEvent<HTMLAnchorElement>) => {
      event.preventDefault();
      openSettings();
    },
    [openSettings],
  );

  const keysAndValues = Object.entries(features ?? {});

  return (
    <Dialog open={success} onOpenChange={reset}>
      <Dialog.Content>
        <Dialog.Title>Hidden Features Menu</Dialog.Title>
        <Dialog.Description>
          Toggle experimental features that are not shown in regular settings.
        </Dialog.Description>
        <div className={`${styles.body} rf-enter`}>
          {keysAndValues.length === 0 ? (
            <p className={styles.empty}>No hidden features</p>
          ) : (
            <SettingItem
              className="rf-enter"
              title="Feature flags"
              description="Some flags are managed by the main settings screen and are locked here."
              layout="stack"
            >
              <div className={`${styles.featureList} rf-stagger`}>
                {keysAndValues.map(([feature, value]) => {
                  const setInSettings =
                    feature === "ast" ||
                    feature === "vecdb" ||
                    feature === "codegraph";
                  return (
                    <div
                      className={`${styles.featureRow} rf-enter`}
                      key={feature}
                    >
                      <div className={styles.featureCopy}>
                        <span className={styles.featureName}>{feature}</span>
                        {setInSettings ? (
                          <span className={styles.featureHint}>
                            Option set in{" "}
                            <Link onClick={handleSettingsClick}>settings</Link>
                          </span>
                        ) : null}
                      </div>
                      <FieldSwitch
                        checked={value}
                        onChange={() =>
                          dispatch(changeFeature({ feature, value: !value }))
                        }
                        disabled={setInSettings}
                      />
                    </div>
                  );
                })}
              </div>
            </SettingItem>
          )}

          <div className={styles.actions}>
            <Dialog.Close asChild>
              <Button variant="ghost">Close</Button>
            </Dialog.Close>
          </div>
        </div>
      </Dialog.Content>
    </Dialog>
  );
};
