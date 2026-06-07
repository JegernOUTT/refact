export declare function hasProperty<T extends string>(obj: object, prop: T): obj is {
    [K in T]: unknown;
};
