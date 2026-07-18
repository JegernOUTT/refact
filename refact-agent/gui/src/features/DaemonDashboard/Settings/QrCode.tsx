import { useEffect, useState } from "react";

import { LoadingState } from "../../../components/ui";
import styles from "./SettingsPage.module.css";

export type QrCodeProps = {
  url: string;
};

export function QrCode({ url }: QrCodeProps) {
  const [svg, setSvg] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let active = true;
    setSvg(null);
    setFailed(false);

    void import("qrcode")
      .then((qrcode) =>
        qrcode.toString(url, {
          type: "svg",
          errorCorrectionLevel: "M",
          margin: 1,
        }),
      )
      .then((nextSvg) => {
        if (active) setSvg(nextSvg);
      })
      .catch(() => {
        if (active) setFailed(true);
      });

    return () => {
      active = false;
    };
  }, [url]);

  if (failed) {
    return <span className={styles.muted}>QR code unavailable.</span>;
  }

  if (!svg) {
    return <LoadingState kind="spinner" label="Generating QR code" />;
  }

  return (
    <div
      aria-label={`QR code for ${url}`}
      className={styles.qrCode}
      dangerouslySetInnerHTML={{ __html: svg }}
      role="img"
    />
  );
}
