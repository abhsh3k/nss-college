/** Production Tailwind config. Keep in sync with templates/partials/styles.html (dev CDN config). */
module.exports = {
  // Templates and the client script are the usual sources; `src/**/*.rs` is
  // included because shared fragments with Tailwind classes are built in Rust
  // (see `src/ui.rs`), and Tailwind must see those class names to emit them.
  content: ["./templates/**/*.html", "./static/js/**/*.js", "./src/**/*.rs"],
  theme: {
    extend: {
      colors: {
        ink: "#2A1410",
        deep: "#5E1A14",
        brand: "#B3261E",
        mist: "#FBEAE0",
        chalk: "#FFFAF6",
        accent: "#F28C28",
      },
      fontFamily: {
        display: ['"Fraunces"', "Georgia", "serif"],
        sans: ['"Public Sans"', "system-ui", "sans-serif"],
      },
    },
  },
};
