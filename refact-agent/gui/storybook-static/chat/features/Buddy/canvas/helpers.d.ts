export declare function fillPixel(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, color: string): void;
export declare function fillRow(ctx: CanvasRenderingContext2D, x: number, y: number, pattern: string, colorMap: Record<string, string>): void;
export declare function fillRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, color: string): void;
export declare function strokeEllipse(ctx: CanvasRenderingContext2D, x: number, y: number, radiusX: number, radiusY: number, color: string, lineWidth?: number): void;
export declare function strokeArc(ctx: CanvasRenderingContext2D, x: number, y: number, radius: number, startAngle: number, endAngle: number, color: string, lineWidth?: number): void;
export declare function fillText(ctx: CanvasRenderingContext2D, text: string, x: number, y: number, size: number, color: string, align?: CanvasTextAlign): void;
