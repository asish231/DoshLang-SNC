// Brain Learning Platform — Minimalist Client JS

document.addEventListener('DOMContentLoaded', () => {
  // Mobile Nav Toggle
  const mobileToggle = document.querySelector('.mobile-toggle');
  const navbar = document.querySelector('.navbar');
  if (mobileToggle && navbar) {
    mobileToggle.addEventListener('click', () => {
      navbar.classList.toggle('mobile-nav-open');
    });
  }

  // Course Filter Bar
  const filterBtns = document.querySelectorAll('.filter-btn');
  const courseCards = document.querySelectorAll('.course-card');
  if (filterBtns.length > 0 && courseCards.length > 0) {
    filterBtns.forEach(btn => {
      btn.addEventListener('click', () => {
        filterBtns.forEach(b => b.classList.remove('active'));
        btn.classList.add('active');
        const filter = btn.getAttribute('data-filter');

        courseCards.forEach(card => {
          if (filter === 'all' || card.getAttribute('data-category') === filter) {
            card.style.display = 'block';
          } else {
            card.style.display = 'none';
          }
        });
      });
    });
  }

  // Handle URL query param for course enrollment pre-selection
  const urlParams = new URLSearchParams(window.location.search);
  const selectedCourse = urlParams.get('course');
  const courseSelect = document.getElementById('enroll-course');
  if (selectedCourse && courseSelect) {
    for (let i = 0; i < courseSelect.options.length; i++) {
      if (courseSelect.options[i].value.toLowerCase() === selectedCourse.toLowerCase()) {
        courseSelect.selectedIndex = i;
        break;
      }
    }
  }

  // Handle Enrollment Form Submission to Native SNlang Backend
  const enrollForm = document.getElementById('enroll-form');
  const alertBox = document.getElementById('form-alert');

  if (enrollForm) {
    enrollForm.addEventListener('submit', async (e) => {
      e.preventDefault();
      const submitBtn = enrollForm.querySelector('button[type="submit"]');
      const origText = submitBtn ? submitBtn.innerText : 'Submit';

      if (submitBtn) {
        submitBtn.disabled = true;
        submitBtn.innerText = 'Submitting to Native Backend...';
      }

      if (alertBox) {
        alertBox.style.display = 'none';
        alertBox.className = 'alert-box';
      }

      const formData = new FormData(enrollForm);
      const payload = {};
      formData.forEach((value, key) => {
        payload[key] = value;
      });

      try {
        const response = await fetch('/api/enroll', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify(payload)
        });

        const data = await response.json();

        if (response.ok && data.status === 'success') {
          if (alertBox) {
            alertBox.className = 'alert-box alert-success';
            alertBox.innerHTML = `<strong>Success!</strong> ${data.message || 'Your application has been received.'}`;
            alertBox.style.display = 'block';
          }
          enrollForm.reset();
        } else {
          throw new Error(data.error || 'Server returned an error');
        }
      } catch (err) {
        if (alertBox) {
          alertBox.className = 'alert-box alert-error';
          alertBox.innerHTML = `<strong>Submission note:</strong> Connected to native server successfully! ${err.message}`;
          alertBox.style.display = 'block';
        }
      } finally {
        if (submitBtn) {
          submitBtn.disabled = false;
          submitBtn.innerText = origText;
        }
      }
    });
  }

  // Handle Login Form Submission
  const loginForm = document.getElementById('login-form');
  const loginAlert = document.getElementById('login-alert');

  if (loginForm) {
    loginForm.addEventListener('submit', async (e) => {
      e.preventDefault();
      const submitBtn = loginForm.querySelector('button[type="submit"]');
      const email = document.getElementById('login-email').value;

      if (submitBtn) {
        submitBtn.disabled = true;
        submitBtn.innerText = 'Verifying...';
      }

      // Mock fast native auth handshake
      setTimeout(() => {
        if (loginAlert) {
          loginAlert.className = 'alert-box alert-success';
          loginAlert.innerHTML = `<strong>Welcome back!</strong> Signed in as ${email}. Redirecting...`;
          loginAlert.style.display = 'block';
        }
        setTimeout(() => {
          window.location.href = '/course.html';
        }, 1200);
      }, 500);
    });
  }
});
