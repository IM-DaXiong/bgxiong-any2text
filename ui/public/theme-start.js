try {
  const theme = localStorage.getItem('any2text.theme');
  if (theme) document.documentElement.dataset.theme = theme;
} catch { /* Use the default stylesheet. */ }
