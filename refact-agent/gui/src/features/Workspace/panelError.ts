export function panelErrorDescription(error: unknown): string {
  const record =
    typeof error === "object" && error !== null ? error : undefined;
  const status = record && "status" in record ? record.status : undefined;
  const data = record && "data" in record ? record.data : undefined;
  const code =
    typeof data === "object" && data !== null && "code" in data
      ? data.code
      : undefined;
  if (status === 401 || code === "unauthorized")
    return "Authentication is required. Check the connection credentials, then refresh.";
  if (status === 403 || code === "scope_denied")
    return "Access is denied for this scope. Check the project and privacy settings, then refresh.";
  if (code === "policy_unavailable")
    return "Privacy policy could not be checked. Data is hidden until the policy is available. Refresh to try again.";
  if (code === "cursor_expired" || status === 409)
    return "This snapshot has expired. Refresh to request the first page again.";
  if (status === 404 || code === "not_found_or_not_visible")
    return "This view is not available in the current scope. Check the selected project or event, then refresh.";
  if (
    status === 400 ||
    code === "invalid_argument" ||
    code === "invalid_cursor"
  )
    return "The request could not be accepted. Check the selected scope or event, then refresh.";
  if (status === "CUSTOM_ERROR" || status === "PARSING_ERROR")
    return "The engine returned an unreadable snapshot. Refresh to try again; if it persists, check that the GUI and engine versions match.";
  if (status === 503 || code === "unavailable")
    return "The engine is not ready to provide this view. Wait for the connection to be ready, then refresh.";
  return "The connection could not load this view. Check the engine connection, then refresh to try again.";
}

export function isExpiredCursor(error: unknown): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    "status" in error &&
    error.status === 409
  );
}
