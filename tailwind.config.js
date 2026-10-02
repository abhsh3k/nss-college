/** Production Tailwind config. Keep in sync with templates/partials/styles.html (dev CDN config). */
module.exports = {
  content: ["./templates/**/*.html", "./static/js/**/*.js"],
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
