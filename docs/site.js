const copyButton = document.querySelector("[data-copy]");
const installCommand = "curl -fsSL https://raw.githubusercontent.com/antoineMoPa/moon-dev-tools/main/install.sh | sh";

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
