// beat(n): show every element whose data-beat is n or lower (record.mjs calls it per sentence).
window.beat = (n) => document.querySelectorAll('[data-beat]').forEach((e) => {
  if (Number(e.dataset.beat) <= n) e.classList.add('on');
});
