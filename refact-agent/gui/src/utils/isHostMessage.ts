export function isHostMessage(event: MessageEvent): boolean {
  return (
    event.source === null ||
    event.source === window ||
    event.source === window.parent
  );
}
