import { useCallback, useEffect } from "react";
import { useAppSelector } from "./useAppSelector";
import { useAppDispatch } from "./useAppDispatch";
import {
  selectThreadImagesById,
  selectThreadTextFilesById,
  addThreadImage,
  removeThreadImageByIndex,
  resetThreadImages,
  addThreadTextFile,
  removeThreadTextFileByIndex,
  resetThreadTextFiles,
  type ImageFile,
  type TextFile,
} from "../features/Chat";
import { setError } from "../features/Errors/errorsSlice";
import { setInformation } from "../features/Errors/informationSlice";
import { useCapsForToolUse } from "./useCapsForToolUse";
import { useThreadId } from "../features/Chat/Thread";
import {
  attachmentFileError,
  isSupportedImageFile,
  MAX_ATTACHED_IMAGES,
} from "../utils/attachmentFiles";

export function useAttachedImages() {
  const chatId = useThreadId();
  const images = useAppSelector((state) =>
    selectThreadImagesById(state, chatId),
  );
  const textFiles = useAppSelector((state) =>
    selectThreadTextFilesById(state, chatId),
  );
  const { isMultimodalitySupportedForCurrentModel, data: capsData } =
    useCapsForToolUse();
  const dispatch = useAppDispatch();

  const removeImage = useCallback(
    (index: number) => {
      dispatch(removeThreadImageByIndex({ id: chatId, index }));
    },
    [dispatch, chatId],
  );

  const insertImage = useCallback(
    (file: ImageFile) => {
      dispatch(addThreadImage({ id: chatId, image: file }));
    },
    [dispatch, chatId],
  );

  const removeTextFile = useCallback(
    (index: number) => {
      dispatch(removeThreadTextFileByIndex({ id: chatId, index }));
    },
    [dispatch, chatId],
  );

  const insertTextFile = useCallback(
    (file: TextFile) => {
      dispatch(addThreadTextFile({ id: chatId, file }));
    },
    [dispatch, chatId],
  );

  const handleError = useCallback(
    (error: string) => {
      const action = setError(error);
      dispatch(action);
    },
    [dispatch],
  );

  const handleWarning = useCallback(
    (warning: string) => {
      const action = setInformation(warning);
      dispatch(action);
    },
    [dispatch],
  );

  const processAndInsertImages = useCallback(
    (files: File[]) => {
      if (files.length > MAX_ATTACHED_IMAGES) {
        handleError(
          `You can only upload ${MAX_ATTACHED_IMAGES} images at a time`,
        );
        return;
      }
      void processImages(files, insertImage, handleError, handleWarning);
    },
    [handleError, handleWarning, insertImage],
  );

  const processAndInsertTextFiles = useCallback(
    (files: File[]) => {
      void processTextFiles(files, insertTextFile, handleError);
    },
    [handleError, insertTextFile],
  );

  const resetAllTextFiles = useCallback(() => {
    dispatch(resetThreadTextFiles({ id: chatId }));
  }, [dispatch, chatId]);

  useEffect(() => {
    // Only reset once caps have resolved: while they load the multimodality
    // flag is false-by-default and this effect used to wipe the user's
    // attached images on every caps refetch (audit N-40).
    if (!capsData) return;
    if (!isMultimodalitySupportedForCurrentModel) {
      dispatch(resetThreadImages({ id: chatId }));
    }
  }, [capsData, isMultimodalitySupportedForCurrentModel, dispatch, chatId]);

  return {
    images,
    textFiles,
    setError: handleError,
    setWarning: handleWarning,
    insertImage,
    removeImage,
    processAndInsertImages,
    removeTextFile,
    processAndInsertTextFiles,
    resetAllTextFiles,
  };
}

async function processImages(
  files: File[],
  onSuccess: (image: ImageFile) => void,
  onError: (reason: string) => void,
  onAbort: (reason: string) => void,
) {
  for (const file of files) {
    const validationError =
      attachmentFileError(file) ??
      (!isSupportedImageFile(file)
        ? `Could not attach ${file.name}: unsupported image type`
        : null);
    if (validationError) {
      onError(validationError);
      continue;
    }
    try {
      const fileForChat = {
        name: file.name,
        content: await readImageFile(file),
        type: file.type,
      };
      onSuccess(fileForChat);
    } catch (error) {
      if (error === "abort") {
        onAbort(`file ${file.name} reading was aborted`);
      } else {
        onError(`file ${file.name} processing has failed`);
      }
    }
  }
}

function readImageFile(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onabort = () => reject("abort");
    reader.onerror = () => reject("error");
    reader.readAsDataURL(file);
  });
}

async function processTextFiles(
  files: File[],
  onSuccess: (file: TextFile) => void,
  onError: (reason: string) => void,
) {
  for (const file of files) {
    try {
      const content = await readTextFile(file);
      onSuccess({ name: file.name, content });
    } catch (error) {
      onError(`file ${file.name} processing has failed`);
    }
  }
}

function readTextFile(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      resolve(reader.result as string);
    };
    reader.onabort = () => reject("abort");
    reader.onerror = () => reject("error");
    reader.readAsText(file);
  });
}
