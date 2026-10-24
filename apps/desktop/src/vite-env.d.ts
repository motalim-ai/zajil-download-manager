/// <reference types="svelte" />
/// <reference types="vite/client" />

declare const __UI_TEST__: boolean;

declare module "*.ftl?raw" {
  const source: string;
  export default source;
}
