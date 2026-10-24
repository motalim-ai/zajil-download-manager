/// <reference types="svelte" />
/// <reference types="vite/client" />

declare module "*.ftl?raw" {
  const source: string;
  export default source;
}
