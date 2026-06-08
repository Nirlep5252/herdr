import type { HerdrDesktopApi } from "../shared/herdr";

declare global {
  interface Window {
    herdr?: HerdrDesktopApi;
  }
}

export {};
