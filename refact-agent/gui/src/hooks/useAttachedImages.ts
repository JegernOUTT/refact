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

export function useAttachedImages() {
  const chatId = useThreadId();
  const images = useAppSelector((state) =>
    selectThreadImagesById(state, chatId),
  );
  const textFiles = useAppSelector((state) =>
    selectThreadTextFilesById(state, chatId),
  );
  const { isMultimodalitySupportedForCurrentModel } = useCapsForToolUse();
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
      if (files.length > 5) {
        handleError("You can only upload 5 images at a time");
        return;
      } else {
        void processImages(files, insertImage, handleError, handleWarning);
      }
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
    if (!isMultimodalitySupportedForCurrentModel) {
      dispatch(resetThreadImages({ id: chatId }));
    }
  }, [isMultimodalitySupportedForCurrentModel, dispatch, chatId]);

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
    if (file.type !== "image/jpeg" && file.type !== "image/png") {
      onError(`file ${file.type} is not supported. Use jpeg or png`);
    } else {
      try {
        const scaledImage = await scaleImage(file, 800);
        const fileForChat = {
          name: file.name,
          content: scaledImage,
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
function scaleImage(file: File, maxSize: number): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const img = new Image();
      img.onload = () => {
        const canvas = document.createElement("canvas");
        const ctx = canvas.getContext("2d");
        if (ctx === null) {
          reject(`canvas.getContext("2d"), returned null`);
        }

        let width = img.width;
        let height = img.height;

        if (width > height && width > maxSize) {
          height = Math.round((height *= maxSize / width));
          width = maxSize;
        } else if (height >= width && height > maxSize) {
          width = Math.round((width *= maxSize / height));
          height = maxSize;
        }

        canvas.width = width;
        canvas.height = height;
        ctx?.drawImage(img, 0, 0, width, height);

        resolve(canvas.toDataURL(file.type));
      };
      img.onerror = reject;
      img.src = reader.result as string;
    };

    reader.onabort = () => reject("aborted");
    reader.onerror = () => reject("error");
    reader.readAsDataURL(file);
  });
}
