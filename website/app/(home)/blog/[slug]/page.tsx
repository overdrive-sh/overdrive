import Link from "next/link";
import { notFound } from "next/navigation";
import { InlineTOC } from "fumadocs-ui/components/inline-toc";
import { publishedBlogPages } from "@/lib/source";
import { getMDXComponents } from "@/mdx-components";

// Blog post page (US-07). Hand-rolled — renders the post body with the same MDX
// component set the docs use, plus an `InlineTOC` (no docs sidebar; this is the
// `(home)` shell). Resolution goes through `publishedBlogPages()` — the single
// draft gate (DoR 3rd UAT scenario): a `draft: true` post is NOT in the
// published set, so `/blog/<draft-slug>` returns 404 (the documented choice —
// drafts are unreachable from every surface, not soft-hidden). `generateStaticParams`
// likewise enumerates only published posts, so no draft is prerendered.

function postDate(value: string | Date): Date {
	return value instanceof Date ? value : new Date(value);
}

function findPublished(slug: string) {
	return publishedBlogPages().find((page) => page.url === `/blog/${slug}`);
}

export default async function BlogPostPage(props: {
	params: Promise<{ slug: string }>;
}) {
	const { slug } = await props.params;
	const page = findPublished(slug);
	if (!page) notFound();

	const Body = page.data.body;
	const date = postDate(page.data.date);

	return (
		<main className="container mx-auto max-w-3xl px-4 py-12">
			<Link
				href="/blog"
				className="text-sm text-fd-muted-foreground hover:underline"
			>
				← All posts
			</Link>

			<article className="mt-6">
				<h1 className="mb-2 text-3xl font-bold">{page.data.title}</h1>
				<div className="mb-6 text-sm text-fd-muted-foreground">
					<time dateTime={date.toISOString()}>
						{date.toLocaleDateString("en-US", {
							year: "numeric",
							month: "long",
							day: "numeric",
						})}
					</time>
					{page.data.author ? <span> · {page.data.author}</span> : null}
				</div>

				{page.data.toc.length > 0 ? (
					<InlineTOC items={page.data.toc} className="mb-8" />
				) : null}

				<div className="prose">
					<Body components={getMDXComponents()} />
				</div>
			</article>
		</main>
	);
}

export async function generateStaticParams() {
	return publishedBlogPages().map((page) => ({
		slug: page.url.replace(/^\/blog\//, ""),
	}));
}

export async function generateMetadata(props: {
	params: Promise<{ slug: string }>;
}) {
	const { slug } = await props.params;
	const page = findPublished(slug);
	if (!page) notFound();
	return {
		title: `${page.data.title} — Overdrive Blog`,
		description: page.data.summary ?? page.data.description,
	};
}
