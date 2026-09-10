"use client";

import Image from "next/image";
import { useEffect, useRef } from "react";
import styles from "./brand.module.css";

type BrandProps = {
	className?: string;
};

const LETTERS = ["o", "v", "e", "r", "d", "r", "i", "v", "e"] as const;
const FOOTER_GAP = 48;
const IMPELLER_DIMENSION = 1254;

type LetterLayout = {
	element: HTMLSpanElement;
	left: number;
	width: number;
	centerY: number;
};

type ScrollLayout = {
	rotor: HTMLImageElement;
	centerX: number;
	centerY: number;
	letters: LetterLayout[];
};

function joinClasses(...classNames: Array<string | undefined>) {
	return classNames.filter(Boolean).join(" ");
}

function clamp(value: number) {
	return Math.max(0, Math.min(1, value));
}

export function ScrollLogo({ className }: BrandProps) {
	const rootRef = useRef<HTMLSpanElement>(null);
	const rotorRef = useRef<HTMLImageElement>(null);
	const wordRef = useRef<HTMLSpanElement>(null);

	useEffect(() => {
		const root = rootRef.current!;
		const rotor = rotorRef.current!;
		const word = wordRef.current!;
		if (!root || !rotor || !word) return;

		const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
		const letters = Array.from(
			word.querySelectorAll<HTMLSpanElement>("[data-brand-letter]"),
		);
		let layout: ScrollLayout | undefined;
		let progress = 0;
		let target = 0;
		let frame = 0;
		let previousTime = 0;
		let disposed = false;

		function measure() {
			const savedTransform = rotor.style.transform;
			rotor.style.transform = "none";
			const rotorBox = rotor.getBoundingClientRect();
			const transformOrigin = getComputedStyle(rotor).transformOrigin
				.split(/\s+/)
				.map(Number.parseFloat);
			const originX = Number.isFinite(transformOrigin[0])
				? transformOrigin[0]
				: rotorBox.width * 0.49497;
			const originY = Number.isFinite(transformOrigin[1])
				? transformOrigin[1]
				: rotorBox.height * 0.51521;
			const wordBox = word.getBoundingClientRect();
			rotor.style.transform = savedTransform;

			layout = {
				rotor,
				centerX: rotorBox.left + originX - wordBox.left,
				centerY: rotorBox.top + originY - wordBox.top,
				letters: letters.map((letter) => ({
					element: letter,
					left: letter.offsetLeft,
					width: letter.offsetWidth,
					centerY: letter.offsetTop + letter.offsetHeight / 2,
				})),
			};
		}

		function render(value: number) {
			const amount = reducedMotion.matches ? 0 : clamp(value);
			const currentLayout = layout;
			if (!currentLayout) return;

			const count = currentLayout.letters.length;
			const sequence = amount * count;
			const consumed = Math.min(count, Math.floor(sequence));
			const local = sequence - consumed;
			const active = currentLayout.letters[consumed];
			const packed = active ? active.left : 0;
			const next = currentLayout.letters[consumed + 1];
			const advance = active
				? next
					? next.left - active.left
					: active.width
				: 0;
			const feed = local * local * (3 - 2 * local);

			currentLayout.rotor.style.transform = `rotate(${amount * count * 360}deg)`;

			currentLayout.letters.forEach((letter, index) => {
				const baseX = letter.left + letter.width / 2;
				let x = -packed - advance * feed;
				let y = 0;
				let scale = 1;
				let angle = 0;
				let opacity = 1;

				if (index < consumed) {
					x = currentLayout.centerX - baseX;
					y = currentLayout.centerY - letter.centerY;
					scale = 0;
					opacity = 0;
				} else if (index === consumed && local > 0) {
					// Only the leading letter is drawn inward. The rest advance intact.
					const startX = baseX - packed;
					const pull = Math.pow(local, 1.25);
					const radius = (startX - currentLayout.centerX) * (1 - pull);
					const curl = local * local * 1.15;
					x = currentLayout.centerX + radius * Math.cos(curl) - baseX;
					y =
						(currentLayout.centerY - letter.centerY) * pull +
						radius * Math.sin(curl) * 0.55;
					scale = Math.pow(1 - local, 1.35);
					angle = (curl * 180) / Math.PI;
					opacity = 1 - clamp((local - 0.82) / 0.18);
				}

				letter.element.style.transform = `translate(${x}px, ${y}px) rotate(${angle}deg) scale(${scale})`;
				letter.element.style.opacity = String(opacity);
			});
		}

		function scrollProgress() {
			const scrollTop = window.scrollY || window.pageYOffset || 0;
			const footer = document.querySelector<HTMLElement>("[data-brand-footer]");
			const documentHeight = Math.max(
				document.documentElement.scrollHeight,
				document.body?.scrollHeight ?? 0,
			);
			const endpoint = footer
				? footer.getBoundingClientRect().top +
					scrollTop -
					window.innerHeight -
					FOOTER_GAP
				: documentHeight - window.innerHeight;

			return clamp(scrollTop / Math.max(1, endpoint));
		}

		function tick(time: number) {
			if (disposed) return;
			frame = 0;
			const elapsed = previousTime
				? Math.min(64, time - previousTime)
				: 16;
			previousTime = time;
			progress += (target - progress) * (1 - Math.exp(-elapsed / 75));
			if (Math.abs(target - progress) < 0.00005) progress = target;
			render(progress);
			if (progress !== target) {
				frame = window.requestAnimationFrame(tick);
			} else {
				previousTime = 0;
			}
		}

		function onScroll() {
			target = reducedMotion.matches ? 0 : scrollProgress();
			if (!frame) frame = window.requestAnimationFrame(tick);
		}

		function refresh() {
			if (disposed) return;
			if (frame) window.cancelAnimationFrame(frame);
			frame = 0;
			previousTime = 0;
			measure();
			target = reducedMotion.matches ? 0 : scrollProgress();
			progress = target;
			render(progress);
		}

		const onReducedMotionChange = () => refresh();
		const onImageLoad = () => refresh();
		window.addEventListener("scroll", onScroll, { passive: true });
		window.addEventListener("resize", refresh, { passive: true });
		window.addEventListener("pageshow", refresh);
		reducedMotion.addEventListener("change", onReducedMotionChange);
		rotor.addEventListener("load", onImageLoad);

		let resizeObserver: ResizeObserver | undefined;
		if (typeof ResizeObserver !== "undefined") {
			resizeObserver = new ResizeObserver(refresh);
			resizeObserver.observe(root);
			resizeObserver.observe(word);
			if (document.body) resizeObserver.observe(document.body);
		}

		void document.fonts?.ready.then(() => {
			if (!disposed) refresh();
		});
		refresh();

		return () => {
			disposed = true;
			if (frame) window.cancelAnimationFrame(frame);
			window.removeEventListener("scroll", onScroll);
			window.removeEventListener("resize", refresh);
			window.removeEventListener("pageshow", refresh);
			reducedMotion.removeEventListener("change", onReducedMotionChange);
			rotor.removeEventListener("load", onImageLoad);
			resizeObserver?.disconnect();
		};
	}, []);

	return (
		<span
			ref={rootRef}
			className={joinClasses(styles.lockup, className)}
			role="img"
			aria-label="Overdrive"
			data-intake-logo
		>
			<Image
				ref={rotorRef}
				className={styles.rotor}
				src="/impeller.png"
				alt=""
				aria-hidden="true"
				width={IMPELLER_DIMENSION}
				height={IMPELLER_DIMENSION}
				draggable={false}
			/>
			<span ref={wordRef} className={styles.word} aria-hidden="true">
				{LETTERS.map((letter, index) => (
					<span
						key={`${letter}-${index}`}
						className={styles.letter}
						data-brand-letter
					>
						{letter}
					</span>
				))}
			</span>
		</span>
	);
}

export function HoverImpeller({ className }: BrandProps) {
	const controlRef = useRef<HTMLButtonElement>(null);
	const rotorRef = useRef<HTMLImageElement>(null);
	const hoveringRef = useRef(false);
	const pressingRef = useRef(false);
	const keyboardFocusedRef = useRef(false);
	const speedRef = useRef(0);
	const angleRef = useRef(0);
	const frameRef = useRef(0);
	const previousTimeRef = useRef<number | null>(null);

	useEffect(() => {
		const control = controlRef.current!;
		const rotor = rotorRef.current!;
		if (!control || !rotor) return;

		const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
		let disposed = false;
		const topSpeed = 1800;
		const spoolTime = 2.8;
		const coastTime = 0.85;

		const powered = () =>
			hoveringRef.current ||
			pressingRef.current ||
			keyboardFocusedRef.current;

		function tick(time: number) {
			if (disposed) return;
			frameRef.current = 0;
			const previousTime = previousTimeRef.current;
			const elapsed =
				previousTime === null
					? 0
					: Math.min((time - previousTime) / 1000, 0.064);
			previousTimeRef.current = time;
			const target = powered() ? topSpeed : 0;
			const duration = powered() ? spoolTime : coastTime;
			const decay = Math.exp(-elapsed / duration);

			// Integrate changing speed so entering/leaving never resets the angle.
			angleRef.current =
				(angleRef.current +
					target * elapsed +
					(speedRef.current - target) * duration * (1 - decay)) %
				360;
			speedRef.current = target + (speedRef.current - target) * decay;
			rotor.style.transform = `rotate(${angleRef.current}deg)`;

			if (powered() || speedRef.current >= 1) {
				frameRef.current = window.requestAnimationFrame(tick);
			} else {
				speedRef.current = 0;
				previousTimeRef.current = null;
			}
		}

		function wake() {
			if (reducedMotion.matches || frameRef.current) return;
			previousTimeRef.current = null;
			frameRef.current = window.requestAnimationFrame(tick);
		}

		const onPointerEnter = (event: PointerEvent) => {
			if (event.pointerType === "touch") return;
			hoveringRef.current = true;
			wake();
		};
		const onPointerLeave = (event: PointerEvent) => {
			if (event.pointerType === "touch") return;
			hoveringRef.current = false;
			wake();
		};
		const onPointerDown = (event: PointerEvent) => {
			if (event.pointerType !== "touch") return;
			pressingRef.current = true;
			if (typeof control.setPointerCapture === "function") {
				control.setPointerCapture(event.pointerId);
			}
			wake();
		};
		const release = () => {
			pressingRef.current = false;
			wake();
		};
		const onFocus = () => {
			keyboardFocusedRef.current = control.matches(":focus-visible");
			wake();
		};
		const onBlur = () => {
			keyboardFocusedRef.current = false;
			wake();
		};
		const onReducedMotionChange = () => {
			if (frameRef.current) window.cancelAnimationFrame(frameRef.current);
			frameRef.current = 0;
			previousTimeRef.current = null;
			speedRef.current = 0;
			if (!reducedMotion.matches && powered()) wake();
		};

		control.addEventListener("pointerenter", onPointerEnter);
		control.addEventListener("pointerleave", onPointerLeave);
		control.addEventListener("pointerdown", onPointerDown);
		control.addEventListener("pointerup", release);
		control.addEventListener("pointercancel", release);
		control.addEventListener("lostpointercapture", release);
		control.addEventListener("focus", onFocus);
		control.addEventListener("blur", onBlur);
		reducedMotion.addEventListener("change", onReducedMotionChange);

		return () => {
			disposed = true;
			if (frameRef.current) window.cancelAnimationFrame(frameRef.current);
			frameRef.current = 0;
			previousTimeRef.current = null;
			control.removeEventListener("pointerenter", onPointerEnter);
			control.removeEventListener("pointerleave", onPointerLeave);
			control.removeEventListener("pointerdown", onPointerDown);
			control.removeEventListener("pointerup", release);
			control.removeEventListener("pointercancel", release);
			control.removeEventListener("lostpointercapture", release);
			control.removeEventListener("focus", onFocus);
			control.removeEventListener("blur", onBlur);
			reducedMotion.removeEventListener("change", onReducedMotionChange);
			hoveringRef.current = false;
			pressingRef.current = false;
			keyboardFocusedRef.current = false;
		};
	}, []);

	return (
		<button
			ref={controlRef}
			className={joinClasses(styles.hoverButton, className)}
			type="button"
			aria-label="Spin the Overdrive impeller"
			data-hover-impeller
		>
			<Image
				ref={rotorRef}
				className={styles.hoverRotor}
				src="/impeller.png"
				alt=""
				aria-hidden="true"
				width={IMPELLER_DIMENSION}
				height={IMPELLER_DIMENSION}
				draggable={false}
			/>
		</button>
	);
}
