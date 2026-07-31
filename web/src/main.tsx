import { createRoot } from "react-dom/client";
import "highlight.js/styles/github.css";
import { Toaster } from "sonner";
import "./app.css";
import "./fullFileView.css";
import { Workspace } from "./components/workspace/Workspace";
import { WorkspaceProvider } from "./components/workspace/workspaceState";
import { ReviewStoreProvider } from "./reviewStore";
import { ThemeProvider, useTheme } from "./theme";

/// The workspace fills the window; the app header lives in the top-left frame's tab strip.
function AppShell() {
  const { theme } = useTheme();

  return (
    <>
      <Toaster closeButton position="bottom-right" richColors theme={theme} />
      <Workspace />
    </>
  );
}

function App() {
  return (
    <ReviewStoreProvider>
      <ThemeProvider>
        <WorkspaceProvider>
          <AppShell />
        </WorkspaceProvider>
      </ThemeProvider>
    </ReviewStoreProvider>
  );
}

createRoot(document.getElementById("app")!).render(<App />);
