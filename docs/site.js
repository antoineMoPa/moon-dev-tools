const copyButton = document.querySelector("[data-copy]");
const installCommand = "curl -fsSL https://raw.githubusercontent.com/antoineMoPa/moon-dev-tools/main/install.sh | sh";
const brandType = document.querySelector("[data-brand-type]");
const brandWords = ["tasks", "review", "shell", "dev tools"];

if (brandType && !window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
  let wordIndex = 0;
  let characterIndex = brandWords[0].length;
  let deleting = true;

  const typeBrand = () => {
    const word = brandWords[wordIndex];

    if (deleting) {
      characterIndex -= 1;
      brandType.textContent = word.slice(0, characterIndex);
      if (characterIndex === 0) {
        deleting = false;
        wordIndex = (wordIndex + 1) % brandWords.length;
      }
    } else {
      characterIndex += 1;
      brandType.textContent = brandWords[wordIndex].slice(0, characterIndex);
      if (characterIndex === brandWords[wordIndex].length) deleting = true;
    }

    const pause = characterIndex === 0 ? 350 : deleting && characterIndex === brandWords[wordIndex]?.length ? 1500 : deleting ? 65 : 105;
    window.setTimeout(typeBrand, pause);
  };

  window.setTimeout(typeBrand, 1500);
}

copyButton?.addEventListener("click", async () => {
  try {
    await navigator.clipboard.writeText(installCommand);
    const label = copyButton.querySelector(".copy-label");
    if (label) label.textContent = "Copied";
    copyButton.setAttribute("aria-label", "Install command copied");
    window.setTimeout(() => {
      if (label) label.textContent = "Copy";
      copyButton.setAttribute("aria-label", "Copy install command");
    }, 1800);
  } catch {
    window.prompt("Copy the install command:", installCommand);
  }
});
