import { AppShell } from "./AppShell";
import { FarmView } from "../features/farm/FarmView";

export default function App() {
  if (window.location.pathname === "/farm") {
    return <FarmView />;
  }
  return <AppShell />;
}
