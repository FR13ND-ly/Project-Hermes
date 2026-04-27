import { ComponentFixture, TestBed } from '@angular/core/testing';

import { Deployments } from './deployments';

describe('Deployments', () => {
  let component: Deployments;
  let fixture: ComponentFixture<Deployments>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [Deployments]
    })
    .compileComponents();

    fixture = TestBed.createComponent(Deployments);
    component = fixture.componentInstance;
    await fixture.whenStable();
  });

  it('should create', () => {
    expect(component).toBeTruthy();
  });
});
