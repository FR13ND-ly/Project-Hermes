import { ComponentFixture, TestBed } from '@angular/core/testing';

import { AuthManagement } from './auth-management';

describe('AuthManagement', () => {
  let component: AuthManagement;
  let fixture: ComponentFixture<AuthManagement>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [AuthManagement]
    })
    .compileComponents();

    fixture = TestBed.createComponent(AuthManagement);
    component = fixture.componentInstance;
    await fixture.whenStable();
  });

  it('should create', () => {
    expect(component).toBeTruthy();
  });
});
