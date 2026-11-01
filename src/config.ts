export const PRODUCT_NAME = "Zap Books";
/**
 * The brand files in public/brand/ (see its README for the usage rules),
 * relative to the page, so they're found wherever the app is served
 * (zapbooks.io/demo/, say).
 */
export const BRAND = {
  /** Full logo, for light surfaces. */
  logo: "brand/zap-books-logo.svg",
  /** Full logo with a white wordmark, for dark surfaces such as the sidebar. */
  logoOnDark: "brand/zap-books-logo-dark-transparent.svg",
  /** The icon alone, wherever there's less than 48 px of room. */
  icon: "brand/zap-books-icon.svg",
};
/** Where the local receipt extractor listens unless VITE_EXTRACTOR_URL says otherwise. */
export const DEFAULT_EXTRACTOR_URL = "http://127.0.0.1:8787";
