import { BrowserRouter as Router, Routes, Route } from "react-router-dom";
import Layout from "./components/Layout";
import Dashboard from "./pages/Dashboard";
import Settings from "./pages/Settings";
import Rules from "./pages/Rules";
import Logs from "./pages/Logs";
import FlowMonitor from "./pages/FlowMonitor";
import Redirect from "./pages/Redirect";

import About from "./pages/About";

function App() {
  return (
    <Router>
      <Layout>
        <Routes>
          <Route path="/" element={<Dashboard />} />
          <Route path="/settings" element={<Settings />} />

          <Route path="/rules" element={<Rules />} />
          <Route path="/logs" element={<Logs />} />
          <Route path="/flows" element={<FlowMonitor />} />
          <Route path="/redirect" element={<Redirect />} />
          <Route path="/about" element={<About />} />
        </Routes>
      </Layout>
    </Router>
  );
}

export default App;
