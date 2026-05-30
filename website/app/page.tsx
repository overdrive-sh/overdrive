import { redirect } from "next/navigation";

// Slice 01: the landing page (US-08) is out of scope. Send `/` to the docs
// skeleton so the root is not a 404. The real HomeLayout lands in slice 08.
export default function HomePage() {
	redirect("/docs");
}
