import React, { useCallback, useEffect, useRef, useState } from "react";
import { ImageEditor } from "../ImageEditor";
import { MAX_ATTACHED_IMAGES } from "../../utils/attachmentFiles";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { setError } from "../../features/Errors/errorsSlice";
import { ImageAttachmentEditorContext, type ImageEditRequest } from "./context";
import { useImageAttachmentScope } from "./useImageAttachmentScope";

type Pending = ImageEditRequest & { id: number; scope: string };

export function ImageAttachmentEditorProvider({
  children,
}: React.PropsWithChildren) {
  const scope = useImageAttachmentScope();
  const dispatch = useAppDispatch();
  const [queue, setQueue] = useState<Pending[]>([]);
  const queueRef = useRef(queue);
  const scopeRef = useRef(scope);
  const nextId = useRef(0);
  const mounted = useRef(true);
  queueRef.current = queue;
  scopeRef.current = scope;

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      queueRef.current = [];
    };
  }, []);
  useEffect(() => {
    queueRef.current = [];
    setQueue([]);
  }, [scope.key]);

  const enqueue = useCallback(
    (requests: ImageEditRequest[]) => {
      if (!mounted.current || !scopeRef.current.allowed) return;
      if (requests.length + queueRef.current.length > MAX_ATTACHED_IMAGES) {
        dispatch(
          setError(
            `You can only upload ${MAX_ATTACHED_IMAGES} images at a time`,
          ),
        );
        return;
      }
      const pending = requests.map((request) => ({
        ...request,
        id: nextId.current++,
        scope: scopeRef.current.key,
      }));
      queueRef.current = [...queueRef.current, ...pending];
      setQueue(queueRef.current);
    },
    [dispatch],
  );

  const current = queue.at(0);
  const finish = (file?: File) => {
    if (
      !current ||
      !mounted.current ||
      queueRef.current[0]?.id !== current.id ||
      current.scope !== scopeRef.current.key ||
      !scopeRef.current.allowed
    )
      return;
    queueRef.current = queueRef.current.slice(1);
    setQueue(queueRef.current);
    if (file) current.accept(file);
  };

  return (
    <ImageAttachmentEditorContext.Provider value={enqueue}>
      {children}
      {current && current.scope === scope.key && scope.allowed && (
        <ImageEditor
          key={current.id}
          file={current.file}
          onApply={(file) => finish(file)}
          onCancel={() => finish()}
        />
      )}
    </ImageAttachmentEditorContext.Provider>
  );
}
