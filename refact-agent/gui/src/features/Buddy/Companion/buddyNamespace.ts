import { getEngineEndpointIdentity } from "../../../services/refact/apiUrl";
import { getProjectStorageNamespace } from "../../../utils/chatUiPersistence";
import type { Config } from "../../Config/configSlice";

export function buddyNamespace(config: Config) {
  return `${getEngineEndpointIdentity(config)}:${config.host}:${
    getProjectStorageNamespace() ?? "unscoped"
  }`;
}
