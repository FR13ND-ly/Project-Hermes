import { ComponentFixture, TestBed } from '@angular/core/testing';

import { Databases } from './databases';

describe('Databases', () => {
  let component: Databases;
  let fixture: ComponentFixture<Databases>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [Databases]
    })
    .compileComponents();

    fixture = TestBed.createComponent(Databases);
    component = fixture.componentInstance;
    await fixture.whenStable();
  });

  it('should create', () => {
    expect(component).toBeTruthy();
  });
});
