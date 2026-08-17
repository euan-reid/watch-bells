const WATCH_NAMES = [
  "middle",
  "morning",
  "forenoon",
  "afternoon",
  "first dog",
  "last dog",
  "first",
];

const BELL_NAMES = [
  "",
  "one bell",
  "two bells",
  "three bells",
  "four bells",
  "five bells",
  "six bells",
  "seven bells",
  "eight bells",
];

const BELL_PATTERNS = [
  "",
  "•",
  "••",
  "••  •",
  "••  ••",
  "••  ••  •",
  "••  ••  ••",
  "••  ••  ••  •",
  "••  ••  ••  ••",
];

const twoDigits = (number) => String(number).padStart(2, "0");

const formatTime = (date) => `${twoDigits(date.getHours())}:${twoDigits(date.getMinutes())}`;

const watchIndexForHour = (hour) => {
  if (hour < 4) return 0;
  if (hour < 8) return 1;
  if (hour < 12) return 2;
  if (hour < 16) return 3;
  if (hour < 18) return 4;
  if (hour < 20) return 5;
  return 6;
};

const bellStateAtBoundary = (boundary) => {
  const hour = boundary.getHours();
  const watchIndex = watchIndexForHour(hour);
  let bells = (hour % 4) * 2 + (boundary.getMinutes() === 30 ? 1 : 0);

  if (bells === 0) bells = 8;
  if (watchIndex === 5 && bells >= 5 && bells !== 8) bells -= 4;

  return { watch: WATCH_NAMES[watchIndex], bells };
};

const surroundingBoundaries = (now) => {
  const latest = new Date(now);
  latest.setSeconds(0, 0);
  latest.setMinutes(latest.getMinutes() < 30 ? 0 : 30);

  const next = new Date(latest);
  next.setMinutes(next.getMinutes() + 30);

  return { latest, next };
};

const liveBells = document.querySelector("#live-bells");

const updateLiveBells = () => {
  if (!liveBells) return;

  const { latest, next } = surroundingBoundaries(new Date());
  const { watch, bells } = bellStateAtBoundary(latest);

  liveBells.textContent = [
    `${watch} watch`,
    BELL_NAMES[bells],
    BELL_PATTERNS[bells],
    `last ${formatTime(latest)}`,
    `next ${formatTime(next)}`,
  ].join(" · ");
};

updateLiveBells();
window.setInterval(updateLiveBells, 15_000);
