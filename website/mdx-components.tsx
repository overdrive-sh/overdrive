import defaultMdxComponents from "fumadocs-ui/mdx";
import { Tab, Tabs } from "fumadocs-ui/components/tabs";
import { Accordion, Accordions } from "fumadocs-ui/components/accordion";
import { Step, Steps } from "fumadocs-ui/components/steps";
import type { MDXComponents } from "mdx/types";
import { Mermaid } from "@/components/mermaid";

// `defaultMdxComponents` already provides Callout, Card/Cards, and the
// code-block tab family. Tabs/Accordions/Steps live in separate modules and
// must be wired explicitly so docs authors can use them without per-page
// imports (slice 02 — make the Fumadocs authoring components available).
// Mermaid is a local client component (renders diagrams in-browser).
export function getMDXComponents(components?: MDXComponents): MDXComponents {
	return {
		...defaultMdxComponents,
		Tab,
		Tabs,
		Accordion,
		Accordions,
		Step,
		Steps,
		Mermaid,
		...components,
	};
}
