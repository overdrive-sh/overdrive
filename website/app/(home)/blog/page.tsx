import Link from "next/link";
import { publishedBlogPages } from "@/lib/source";

// Blog list page (US-07). Hand-rolled — Fumadocs ships no `<BlogLayout>`. Lists
// the PUBLISHED posts (title + date) newest-first under the shared `(home)`
// shell. Drafts come from `publishedBlogPages()` already filtered out (DoR 3rd
// UAT scenario) — a `draft: true` post is never listed and has no link here.
export const metadata = {
	title: "Blog — Overdrive",
	description: "Posts from the Overdrive team.",
};

function postDate(value: string | Date): Date {
	return value instanceof Date ? value : new Date(value);
}

export default function BlogIndexPage() {
	const posts = publishedBlogPages()
		.slice()
		.sort(
			(a, b) =>
				postDate(b.data.date).getTime() - postDate(a.data.date).getTime(),
		);

	return (
		<main className="container mx-auto max-w-3xl px-4 py-12">
			<h1 className="mb-2 text-3xl font-bold">Blog</h1>
			<p className="mb-8 text-fd-muted-foreground">
				Notes from the team building Overdrive.
			</p>

			{posts.length === 0 ? (
				<p className="text-fd-muted-foreground">No posts yet.</p>
			) : (
				<ul className="flex flex-col gap-6">
					{posts.map((post) => (
						<li
							key={post.url}
							className="border-b border-fd-border pb-6 last:border-b-0"
						>
							<Link
								href={post.url}
								className="text-xl font-semibold hover:underline"
							>
								{post.data.title}
							</Link>
							<div className="mt-1 text-sm text-fd-muted-foreground">
								<time dateTime={postDate(post.data.date).toISOString()}>
									{postDate(post.data.date).toLocaleDateString("en-US", {
										year: "numeric",
										month: "long",
										day: "numeric",
									})}
								</time>
								{post.data.author ? <span> · {post.data.author}</span> : null}
							</div>
							{post.data.summary ? (
								<p className="mt-2 text-fd-muted-foreground">
									{post.data.summary}
								</p>
							) : post.data.description ? (
								<p className="mt-2 text-fd-muted-foreground">
									{post.data.description}
								</p>
							) : null}
						</li>
					))}
				</ul>
			)}
		</main>
	);
}
