import { ComponentFixture, TestBed } from '@angular/core/testing';

import { Networking } from './networking';

describe('Networking', () => {
  let component: Networking;
  let fixture: ComponentFixture<Networking>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [Networking]
    })
    .compileComponents();

    fixture = TestBed.createComponent(Networking);
    component = fixture.componentInstance;
    await fixture.whenStable();
  });

  it('should create', () => {
    expect(component).toBeTruthy();
  });
});
