import type { AppStore } from "../../app/store";
import {
  removeThreadImageByIndex,
  removeThreadTextFileByIndex,
  selectThreadImagesById,
  selectThreadTextFilesById,
  type ImageFile,
  type TextFile,
} from "../../features/Chat/Thread";

export type SubmittedAttachments = {
  images: ImageFile[];
  textFiles: TextFile[];
};

export function captureSubmittedAttachments(
  store: AppStore,
  chatId: string,
): SubmittedAttachments {
  const state = store.getState();
  return {
    images: selectThreadImagesById(state, chatId),
    textFiles: selectThreadTextFilesById(state, chatId),
  };
}

export function clearSubmittedAttachments(
  store: AppStore,
  chatId: string,
  submitted: SubmittedAttachments,
) {
  const state = store.getState();
  const images = selectThreadImagesById(state, chatId);
  const textFiles = selectThreadTextFilesById(state, chatId);
  for (let index = images.length - 1; index >= 0; index--) {
    if (submitted.images.includes(images[index]))
      store.dispatch(removeThreadImageByIndex({ id: chatId, index }));
  }
  for (let index = textFiles.length - 1; index >= 0; index--) {
    if (submitted.textFiles.includes(textFiles[index]))
      store.dispatch(removeThreadTextFileByIndex({ id: chatId, index }));
  }
}
