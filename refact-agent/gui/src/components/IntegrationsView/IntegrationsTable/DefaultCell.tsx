import { useState, useEffect } from "react";
import type { FocusEvent, KeyboardEvent } from "react";
import { FieldText } from "../../ui";

type DefaultCellProps = {
  initialValue: string;
  onChange: (value: string) => void;
  onKeyPress: (e: KeyboardEvent<HTMLInputElement>) => void;
  "data-row-index"?: number;
  "data-field"?: string;
  "data-next-row"?: string;
};

export const DefaultCell = ({
  initialValue,
  onChange,
  onKeyPress,
  "data-row-index": dataRowIndex,
  "data-field": dataField,
  "data-next-row": dataNextRow,
}: DefaultCellProps) => {
  const [value, setValue] = useState(initialValue);

  const handleBlur = (event: FocusEvent<HTMLInputElement>) => {
    onChange(event.target.value);
  };

  useEffect(() => {
    setValue(initialValue);
  }, [initialValue]);

  return (
    <FieldText
      value={value}
      onChange={setValue}
      onBlur={handleBlur}
      onKeyDown={onKeyPress}
      data-row-index={dataRowIndex}
      data-field={dataField}
      data-next-row={dataNextRow}
    />
  );
};
