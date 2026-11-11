import type { MonthTemperature, Wind } from "../../../api/client";

/** Sixteen compass points clockwise from north; a point's bearing is its index times 22.5°. */
export const DIRECTIONS = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];
export const PRINCIPAL = DIRECTIONS.filter((_, index) => index % 2 === 0);
export const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTH_NAMES = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const WORDS: Record<string, string> = { N: "north", E: "east", S: "south", W: "west" };

export const bearingOf = (direction: string) => DIRECTIONS.indexOf(direction) * 22.5;

/** "SW" becomes "south-west", "NNE" becomes "north-north-east". */
export function directionName(direction: string) {
  if (direction.length === 3) return `${WORDS[direction[0]]}-${WORDS[direction[1]]}-${WORDS[direction[2]]}`;
  return [...direction].map((letter) => WORDS[letter]).join("-");
}

export function nearestPrincipal(bearing: number) {
  return PRINCIPAL[Math.round((((bearing % 360) + 360) % 360) / 45) % 8];
}

/** 1 when an opening faces straight into the wind, 0 side-on, -1 when it faces away. */
export function alignment(openingBearing: number, windBearing: number) {
  return Math.cos(((openingBearing - windBearing) * Math.PI) / 180);
}

const degrees = (value: number) => `${Number.isInteger(value) ? value : value.toFixed(1)} °C`;
const ordinal = (n: number) => `${n}${["th", "st", "nd", "rd"][n % 100 > 10 && n % 100 < 14 ? 0 : Math.min(n % 10, 4) % 4]}`;

/** What the learner reads after choosing part of a figure. */
export interface Explanation {
  title: string;
  /** Statements that follow from the data alone. */
  facts: string[];
  /** A design principle. It is general knowledge, not a quotation from the book. */
  principle: string;
}

export function explainWind(wind: Wind, winds: Wind[]): Explanation {
  const rank = [...winds].sort((a, b) => b.share - a.share).findIndex((item) => item.direction === wind.direction) + 1;
  return {
    title: `Wind from the ${directionName(wind.direction)}`,
    facts: [
      `It blows from the ${directionName(wind.direction)} ${wind.share}% of the time.`,
      rank === 1
        ? "This is the prevailing wind: the direction it comes from most often in this data."
        : `It is the ${ordinal(rank)} most frequent of the ${winds.length} directions in this data.`,
    ],
    principle:
      "An opening catches the wind most often when it faces a frequent direction. Frequency alone does not say how strong the wind is or in which season it blows, so check those before relying on it.",
  };
}

export function explainMonth(month: MonthTemperature, low: number, high: number): Explanation {
  const band = `the comfort band of ${degrees(low).replace(" °C", "")}–${degrees(high)}`;
  const relation =
    month.min > high
      ? `Even the coolest part of the day is above ${band}.`
      : month.max < low
        ? `Even the warmest part of the day is below ${band}.`
        : month.min >= low && month.max <= high
          ? `The whole day sits inside ${band}.`
          : month.max > high && month.min < low
            ? `Days rise above ${band} and nights fall below it.`
            : month.max > high
              ? `Days rise above ${band}; nights fall back inside it.`
              : `Nights fall below ${band}; days are inside it.`;
  const principle =
    month.max > high && month.min < low
      ? "A large swing between day and night favours heavy construction that stores the night's coolness for the day."
      : month.max > high
        ? "When the air is warmer than comfortable, shade and air movement matter most."
        : month.min < low
          ? "When the air is cooler than comfortable, admit the sun and hold on to heat."
          : "When the air is comfortable, the building mainly needs to stay open to it.";
  return {
    title: MONTH_NAMES[month.month - 1],
    facts: [`Mean maximum ${degrees(month.max)}, mean minimum ${degrees(month.min)}: a daily range of ${degrees(month.max - month.min)}.`, relation],
    principle,
  };
}

export type PartId = "inlet" | "outlet" | "roof" | "vent" | "wall" | "floor";

export const PARTS: Record<PartId, { name: string; text: string }> = {
  inlet: {
    name: "Inlet opening",
    text: "The low, wide opening on the side the wind arrives from. Air enters at body height, where people feel it.",
  },
  outlet: {
    name: "Outlet opening",
    text: "A higher opening on the far side. Air leaving here is what draws the breeze across the room.",
  },
  roof: { name: "Roof and overhang", text: "The roof takes most of the sun's heat. Its overhang shades the walls and openings below." },
  vent: { name: "Ridge vent", text: "Warm air collects under the roof and rises out here, even when there is little wind." },
  wall: { name: "Solid end wall", text: "A wall with no openings. Facing the wind, it blocks the breeze instead of admitting it." },
  floor: { name: "Floor", text: "The occupied zone. The aim is for moving air to cross the room at this level." },
};

export function explainPart(part: PartId): Explanation {
  return { title: PARTS[part].name, facts: [], principle: PARTS[part].text };
}
