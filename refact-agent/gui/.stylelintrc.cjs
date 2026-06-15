module.exports = {
  extends: ["stylelint-config-standard", "stylelint-config-css-modules"],
  rules: {
    "alpha-value-notation": null,
    "color-function-notation": null,
    "color-function-alias-notation": null,
    "block-no-empty": null,
    "value-keyword-case": null,
    "custom-property-pattern": null,
    "selector-class-pattern": null,
    "declaration-property-value-disallowed-list": {
      "/^(?:color|background(?:-color)?|border(?:-(?:top|right|bottom|left))?-color|outline-color|box-shadow|text-shadow|fill|stroke)$/":
        ["/#(?:[0-9a-f]{3,8})\\b/i", "/rgba?\\(/i"],
    },
  },
};
