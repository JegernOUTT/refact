export declare function partition<T, Left extends T = T, Right extends T = T>(array: T[], condition: (a: T) => boolean): (Left[] | Right[])[];
