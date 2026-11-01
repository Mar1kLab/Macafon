import { expect, it } from 'vitest';
import { isLink } from './download';

it('accepts only http(s) links with a host before asking the daemon', () => {
  expect(isLink(' https://www.youtube.com/watch?v=abc ')).toBe(true);
  expect(isLink('http://soundcloud.com/a/b')).toBe(true);
  for (const text of ['', 'youtube.com/watch?v=abc', 'file:///C:/x.mp3', 'javascript:alert(1)', '--exec calc']) {
    expect(isLink(text)).toBe(false);
  }
});
